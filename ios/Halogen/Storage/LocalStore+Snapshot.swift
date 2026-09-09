import Foundation

extension LocalStore {
    /// Cursor publication follows every cache write; a partial projection is replayed on the next boot.
    @discardableResult
    func projectSnapshot(_ json: String?) throws -> Bool {
        guard let json else { return false }
        guard let snapshot = try JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any],
            snapshot["reset_cache"] as? Bool == true, let cursor = snapshot["sync_cursor"] as? String,
            let podcasts = snapshot["podcasts"] as? [[String: Any]],
            let episodes = snapshot["episodes"] as? [[String: Any]],
            let playlists = snapshot["playlists"] as? [[String: Any]],
            let playbacks = snapshot["playbacks"] as? [[String: Any]],
            let autoPlaylists = snapshot["auto_playlists"] as? [String: [[String: Any]]]
        else { throw CocoaError(.coderInvalidValue) }
        guard load(String.self, key: "sync-projected-cursor") != cursor else { return false }
        func id(_ row: [String: Any], _ field: String = "id") -> Int32? { (row[field] as? NSNumber)?.int32Value }
        let podcastMap = Dictionary(uniqueKeysWithValues: podcasts.compactMap { row in id(row).map { ($0, row) } })
        let playbackMap = Dictionary(
            uniqueKeysWithValues: playbacks.compactMap { row in id(row, "episode_id").map { ($0, row) } })
        let episodeMap = Dictionary(
            uniqueKeysWithValues: episodes.compactMap { source -> (Int32, [String: Any])? in
                guard let episodeId = id(source) else { return nil }
                var row = source
                if let podcastId = id(source, "podcast_id") { row["podcast"] = podcastMap[podcastId] ?? NSNull() }
                row["playback"] = playbackMap[episodeId] ?? NSNull()
                return (episodeId, row)
            })
        let ordered = episodeMap.values.sorted {
            String(describing: $0["published_at"] ?? "") > String(describing: $1["published_at"] ?? "")
        }
        let playlistMap = Dictionary(uniqueKeysWithValues: playlists.compactMap { row in id(row).map { ($0, row) } })
        for key in listKeys() {
            if key.hasPrefix("episode-"), let episodeId = Int32(key.dropFirst("episode-".count)) {
                try writeCache(episodeMap[episodeId] ?? NSNull(), key: key, applied: [])
            } else if key.hasPrefix("podcast-episodes-"),
                let podcastId = Int32(key.dropFirst("podcast-episodes-".count))
            {
                try writeCache(ordered.filter { id($0, "podcast_id") == podcastId }, key: key, applied: [])
            } else if key.hasPrefix("playlist-episodes-"),
                let playlistId = Int32(key.dropFirst("playlist-episodes-".count))
            {
                let ids = playlistMap[playlistId]?["episode_ids"] as? [NSNumber] ?? []
                try writeCache(ids.compactMap { episodeMap[$0.int32Value] }, key: key, applied: [])
            } else if key.hasPrefix("auto-playlists-") {
                try writeCache(NSNull(), key: key, applied: [])
            } else if key.hasPrefix("latest-") || key.hasPrefix("downloads-") || key == CacheKey.history {
                if let rows = try readCache(key).payload as? [[String: Any]] {
                    try writeCache(rows.compactMap { id($0).flatMap { episodeMap[$0] } }, key: key, applied: [])
                }
            }
        }
        try writeCache(podcasts, key: CacheKey.podcasts, applied: [])
        try writeCache(playlists, key: CacheKey.playlists, applied: [])
        try writeCache(
            playlists.first { $0["is_default"] as? Bool == true } ?? NSNull(), key: CacheKey.queueMeta, applied: [])
        try writeCache(ordered, key: CacheKey.latest([]), applied: [])
        try writeCache(
            ordered.filter { $0["download_status"] as? String == DownloadStatus.downloaded.rawValue },
            key: CacheKey.downloads("downloaded"), applied: [])
        try writeCache(
            ordered.filter { $0["download_status"] as? String == DownloadStatus.downloading.rawValue },
            key: CacheKey.downloads("downloading"), applied: [])
        let history = playbacks.sorted {
            String(describing: $0["updated_at"] ?? "") > String(describing: $1["updated_at"] ?? "")
        }
        .compactMap { id($0, "episode_id").flatMap { episodeMap[$0] } }
        try writeCache(history, key: CacheKey.history, applied: [])
        try writeCache([], key: CacheKey.podcastTombstones, applied: [])
        for playlist in playlists {
            guard let playlistId = id(playlist) else { continue }
            let ids = playlist["episode_ids"] as? [NSNumber] ?? []
            try writeCache(
                ids.compactMap { episodeMap[$0.int32Value] }, key: CacheKey.playlistEpisodes(playlistId), applied: [])
        }
        for podcast in podcasts {
            guard let podcastId = id(podcast) else { continue }
            try writeCache(
                ordered.filter { id($0, "podcast_id") == podcastId }, key: CacheKey.podcastEpisodes(podcastId),
                applied: [])
        }
        var overlay: [String: Any] = [:]
        for playback in playbacks {
            guard let episodeId = id(playback, "episode_id") else { continue }
            overlay[String(episodeId)] = [
                "cursor": playback["cursor"] ?? 0,
                "completed": playback["completed"] ?? false, "updatedAt": playback["updated_at"] ?? NSNull(),
            ]
        }
        try writeCache(overlay, key: CacheKey.playbacks, applied: [])
        for (podcastId, rows) in autoPlaylists {
            guard let id = Int32(podcastId) else { throw CocoaError(.coderInvalidValue) }
            let payload: [String: Any] = [
                "playlistIds": rows.compactMap { $0["playlist_id"] },
                "addToStart": rows.first?["add_to_start"] ?? NSNull(),
            ]
            try writeCache(payload, key: CacheKey.autoPlaylists(id), applied: [])
        }
        try saveDurably(cursor, key: "sync-projected-cursor")
        return true
    }
}
