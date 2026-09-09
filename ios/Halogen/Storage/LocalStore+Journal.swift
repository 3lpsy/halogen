import Foundation

extension LocalStore {
    /// Project intent before acknowledgement. Each cache carries its own crash-safe replay watermark.
    func projectJournal(_ operations: [OutboxOp]) throws {
        for operation in operations {
            var keys = Set(listKeys()).union([CacheKey.playbacks, CacheKey.playlists, CacheKey.podcastTombstones])
            switch operation.kind {
            case .addToPlaylist(let id, _, _), .removeFromPlaylist(let id, _), .moveInPlaylist(let id, _, _),
                .reorderPlaylist(let id, _, _):
                keys.insert(CacheKey.playlistEpisodes(id))
            default: break
            }
            var removedEpisodes: Set<Int32> = []
            if case .unsubscribe(let podcastId) = operation.kind {
                for key in keys where key != "outbox" && !key.hasPrefix("outbox-") {
                    let payload = try readCache(key).payload
                    let rows = (payload as? [[String: Any]]) ?? (payload as? [String: Any]).map { [$0] } ?? []
                    for row in rows where row["content_url"] != nil && number(row["podcast_id"]) == podcastId {
                        if let id = number(row["id"]) { removedEpisodes.insert(id) }
                    }
                }
            }
            let membershipKeys = [CacheKey.playbacks, CacheKey.playlists, CacheKey.queueMeta]
            let orderedKeys = keys.sorted {
                (membershipKeys.contains($0) ? 0 : 1, $0) < (membershipKeys.contains($1) ? 0 : 1, $1)
            }
            for key in orderedKeys where key != "outbox" && !key.hasPrefix("outbox-") {
                let record = try readCache(key)
                guard !record.applied.contains(operation.id.uuidString) else { continue }
                guard
                    let payload = try project(
                        operation.kind, key: key, payload: record.payload, removedEpisodes: removedEpisodes)
                else { continue }
                try writeCache(payload, key: key, applied: record.applied.union([operation.id.uuidString]))
            }
            if case .setAutoPlaylists(let id, let ids, let position) = operation.kind {
                let key = CacheKey.autoPlaylists(id)
                let record = try readCache(key)
                if !record.applied.contains(operation.id.uuidString) {
                    var payload: [String: Any] = ["playlistIds": ids]
                    payload["addToStart"] = position
                    try writeCache(payload, key: key, applied: record.applied.union([operation.id.uuidString]))
                }
            }
        }
    }

    func forgetJournalMarkers(_ ids: Set<String>) throws {
        for key in listKeys() where key != "outbox" && !key.hasPrefix("outbox-") {
            let record = try readCache(key)
            let retained = record.applied.subtracting(ids)
            if retained != record.applied, let payload = record.payload {
                try writeCache(payload, key: key, applied: retained)
            }
        }
    }

    private func project(
        _ kind: OutboxOp.Kind, key: String, payload: Any?, removedEpisodes: Set<Int32>
    ) throws -> Any? {
        switch kind {
        case .setCursor(let id, let cursor):
            guard key == CacheKey.playbacks else { return nil }
            var rows = payload as? [String: Any] ?? [:]
            var row = rows[String(id)] as? [String: Any] ?? ["completed": false]
            row["cursor"] = cursor
            row["completed"] = false
            row["updatedAt"] = ISO8601DateFormatter().string(from: .now)
            rows[String(id)] = row
            return rows
        case .setPlayed(let id, let played):
            guard key == CacheKey.playbacks else { return nil }
            var rows = payload as? [String: Any] ?? [:]
            rows[String(id)] = [
                "cursor": 0, "completed": played, "updatedAt": ISO8601DateFormatter().string(from: .now),
            ]
            return rows
        case .unsubscribe(let id):
            if key == CacheKey.autoPlaylists(id) || key == CacheKey.metadata("podcasts/\(id)")
                || removedEpisodes.contains(where: { key == CacheKey.metadata("episodes/\($0)") })
            {
                return NSNull()
            }
            if key == CacheKey.podcastTombstones {
                return Array(Set(payload as? [Int32] ?? []).union([id]))
            }
            if key == CacheKey.playbacks, var rows = payload as? [String: Any] {
                for id in removedEpisodes { rows.removeValue(forKey: String(id)) }
                return rows
            }
            return transformRows(payload) { row in
                if row["is_default"] != nil, let ids = row["episode_ids"] as? [NSNumber] {
                    var next = row
                    next["episode_ids"] = ids.filter { !removedEpisodes.contains($0.int32Value) }
                    return next
                }
                if row["feed_url"] != nil && number(row["id"]) == id { return nil }
                if row["content_url"] != nil && number(row["podcast_id"]) == id { return nil }
                return row
            }
        case .removeServerDownload(let id):
            return transformRows(payload) { row in
                guard row["content_url"] != nil && number(row["id"]) == id else { return row }
                if key.hasPrefix("downloads-") { return nil }
                var next = row
                next["download_status"] = DownloadStatus.notDownloaded.rawValue
                for field in ["content_file_path", "downloaded_at", "download_size"] { next[field] = NSNull() }
                return next
            }
        case .updatePlaylist: return try projectPlaylistUpdate(kind, key: key, payload: payload)
        case .movePlaylist(let id, let destination):
            guard key == CacheKey.playlists, var rows = payload as? [[String: Any]] else { return nil }
            rows.sort { (number($0["position"]) ?? 0) < (number($1["position"]) ?? 0) }
            if let index = rows.firstIndex(where: { number($0["id"]) == id }) {
                let row = rows.remove(at: index)
                rows.insert(row, at: min(max(Int(destination), 0), rows.count))
                for index in rows.indices { rows[index]["position"] = index }
            }
            return rows
        case .addToPlaylist(let id, let episode, let position):
            return try membership(key, payload, id: id) { rows in
                guard !rows.contains(where: { number($0["id"]) == episode }) else { return }
                if let cached = try cachedJournalEpisode(episode) {
                    rows.insert(cached, at: min(max(Int(position ?? Int32(rows.count)), 0), rows.count))
                }
            }
        case .removeFromPlaylist(let id, let episode):
            return try membership(key, payload, id: id) { rows in rows.removeAll { number($0["id"]) == episode } }
        case .moveInPlaylist(let id, let episode, let destination):
            return try membership(key, payload, id: id) { rows in
                if let index = rows.firstIndex(where: { number($0["id"]) == episode }) {
                    let row = rows.remove(at: index)
                    rows.insert(row, at: min(max(Int(destination), 0), rows.count))
                }
            }
        case .reorderPlaylist(let id, let field, let direction):
            // Added order is resolved by the server because native episode caches lack pivot timestamps.
            guard field != .added else { return nil }
            return try membership(key, payload, id: id) { rows in
                let fieldKey: String
                switch field {
                case .title: fieldKey = "title"
                case .duration: fieldKey = "duration_secs"
                case .published: fieldKey = "published_at"
                case .added: fieldKey = "added_at"
                }
                rows.sort {
                    let left = $0[fieldKey]
                    let right = $1[fieldKey]
                    if left == nil || left is NSNull { return false }
                    if right == nil || right is NSNull { return true }
                    let less: Bool
                    if field == .duration {
                        less = (number(left) ?? 0) < (number(right) ?? 0)
                    } else {
                        less = String(describing: left!).lowercased() < String(describing: right!).lowercased()
                    }
                    return direction == .asc ? less : !less && String(describing: left!) != String(describing: right!)
                }
            }
        case .updatePodcastConfig(let id, let data):
            let patch = try JSONSerialization.jsonObject(with: WireJSON.encoder.encode(data)) as? [String: Any] ?? [:]
            return transformRows(payload) { row in
                var next = row
                if var config = row["podcast_config"] as? [String: Any], number(config["id"]) == id {
                    config.merge(patch) { _, new in new }; next["podcast_config"] = config
                }
                return next
            }
        case .removePodcastConfig(let id):
            return transformRows(payload) { row in
                guard row["feed_url"] != nil && number(row["id"]) == id else { return row }
                var next = row; next["podcast_config"] = NSNull(); next["podcast_config_id"] = NSNull(); return next
            }
        case .triggerDownload(let id):
            return transformRows(payload) { row in
                guard row["content_url"] != nil && number(row["id"]) == id else { return row }
                var next = row; next["download_status"] = DownloadStatus.downloading.rawValue; return next
            }
        case .subscribe, .setAutoPlaylists: return nil
        }
    }

    private func projectPlaylistUpdate(_ kind: OutboxOp.Kind, key: String, payload: Any?) throws -> Any? {
        guard case .updatePlaylist(let id, let name, let isDefault, let description, let server, let client) = kind
        else { return nil }
        var payload = payload
        if key == CacheKey.queueMeta, isDefault == true,
            let rows = try readCache(CacheKey.playlists).payload as? [[String: Any]],
            let target = rows.first(where: { number($0["id"]) == id })
        {
            payload = target
        }
        return transformRows(payload) { row in
            guard row["is_default"] != nil else { return row }
            var next = row
            if isDefault == true { next["is_default"] = number(row["id"]) == id }
            guard number(row["id"]) == id else { return next }
            if let name { next["name"] = name }
            if let isDefault { next["is_default"] = isDefault }
            if let description { next["description"] = description }
            if let server { next["on_remove_delete_file_server"] = server }
            if let client { next["on_remove_delete_file_client"] = client }
            return next
        }
    }

    private func membership(
        _ key: String, _ payload: Any?, id: Int32,
        mutate: (inout [[String: Any]]) throws -> Void
    ) throws -> Any? {
        if key == CacheKey.playlistEpisodes(id) {
            var rows = payload as? [[String: Any]] ?? []
            try mutate(&rows)
            return rows
        }
        guard key == CacheKey.playlists || key == CacheKey.queueMeta else { return nil }
        func updated(_ row: [String: Any]) throws -> [String: Any] {
            guard number(row["id"]) == id, let ids = row["episode_ids"] as? [NSNumber] else { return row }
            var episodes: [[String: Any]] = try ids.map { episode in
                try cachedJournalEpisode(episode.int32Value) ?? ["id": episode]
            }
            try mutate(&episodes)
            var next = row; next["episode_ids"] = episodes.compactMap { $0["id"] }; return next
        }
        if let rows = payload as? [[String: Any]] { return try rows.map(updated) }
        if let row = payload as? [String: Any] { return try updated(row) }
        return nil
    }

    private func cachedJournalEpisode(_ id: Int32) throws -> [String: Any]? {
        if let row = try readCache(CacheKey.episode(id)).payload as? [String: Any] { return row }
        for key in listKeys() {
            if let rows = try readCache(key).payload as? [[String: Any]],
                let row = rows.first(where: { $0["content_url"] != nil && number($0["id"]) == id })
            {
                return row
            }
        }
        return nil
    }

    private func number(_ value: Any?) -> Int32? { (value as? NSNumber)?.int32Value }

    private func transformRows(_ payload: Any?, transform: ([String: Any]) -> [String: Any]?) -> Any? {
        if let rows = payload as? [[String: Any]] { return rows.compactMap(transform) }
        if let row = payload as? [String: Any] { return transform(row) ?? NSNull() }
        return nil
    }
}
