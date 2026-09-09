import Foundation

/// Playlist slice of the API client. The queue is not a separate concept —
/// it's the user's `is_default` playlist (`GET /playlists/default`).
extension HalogenClient {
    func playlists() async throws -> [PlaylistData] {
        try await get(
            "playlists",
            query: [
                URLQueryItem(name: "includes[0]", value: "EpisodeIds"),
                // Server default page size is 10 — a curated playlist list fits
                // one big page.
                URLQueryItem(name: "pagination[page]", value: "0"),
                URLQueryItem(name: "pagination[size]", value: "500"),
            ])
    }

    /// One playlist by id — the deep-link fetch-through, episode ids
    /// included so the detail can render its list (web `get_playlist`).
    func playlist(id: Int32) async throws -> PlaylistData {
        try await get(
            "playlists/\(id)",
            query: [URLQueryItem(name: "includes[0]", value: "EpisodeIds")])
    }

    /// The queue, or nil when the user has no default playlist yet. The
    /// wrapper (`DefaultPlaylistData`) keeps "no queue" distinct from the
    /// envelope's missing-data error.
    func defaultPlaylist() async throws -> PlaylistData? {
        let wrapper: DefaultPlaylistData = try await get("playlists/default")
        return wrapper.playlist
    }

    /// A playlist's episodes in pivot (position) order, podcast embedded. The
    /// order MUST be requested explicitly: the endpoint's param-less default
    /// is id ASC (front-adds once landed at the END on iOS because of this).
    func playlistEpisodes(playlistId: Int32, pageSize: Int = 500) async throws -> [EpisodeData] {
        try await get(
            "playlists/\(playlistId)/episodes",
            query: [
                URLQueryItem(name: "pagination[page]", value: "0"),
                URLQueryItem(name: "pagination[size]", value: String(pageSize)),
                URLQueryItem(name: "includes[0]", value: "Podcast"),
                URLQueryItem(name: "order[order_by]", value: "position"),
                URLQueryItem(name: "order[direction]", value: "Asc"),
            ])
    }

    /// Create a playlist with the web form's full field set (name +
    /// description + make-default + the two delete-on-remove cleanup flags).
    func createPlaylist(
        name: String, description: String? = nil, isDefault: Bool = false,
        deleteServerFile: Bool = false, deleteClientFile: Bool = false
    ) async throws -> PlaylistData {
        try await post(
            "playlists",
            body: PlaylistStoreData(
                name: name,
                description: description,
                is_default: isDefault,
                on_remove_delete_file_server: deleteServerFile,
                on_remove_delete_file_client: deleteClientFile
            )
        )
    }

    func deletePlaylist(id: Int32) async throws {
        try await delete("playlists/\(id)")
    }

    /// Partial playlist update (`PUT /playlists/{id}`; nil = leave unchanged)
    /// — the edit form's direct path and the outbox drain path for
    /// offline-queued edits (web: `OutboxOp::UpdatePlaylist`).
    func updatePlaylist(
        id: Int32, name: String?, isDefault: Bool?,
        description: String? = nil,
        deleteServerFile: Bool? = nil, deleteClientFile: Bool? = nil
    ) async throws {
        let _: PlaylistData = try await put(
            "playlists/\(id)",
            body: PlaylistUpdateData(
                name: name, description: description, is_default: isDefault,
                on_remove_delete_file_server: deleteServerFile,
                on_remove_delete_file_client: deleteClientFile))
    }

    /// Move a playlist within the user's manual (`position`) order —
    /// `POST /playlists/{id}/move` (web: `commands::move_playlist`).
    func movePlaylist(id: Int32, to: Int32) async throws {
        try await postEmpty("playlists/\(id)/move", body: PlaylistMoveBody(to: to))
    }

    /// Promote a playlist to be the queue (server demotes the old default).
    func makeQueuePlaylist(id: Int32) async throws {
        let _: PlaylistData = try await put(
            "playlists/\(id)",
            body: PlaylistUpdateData(
                name: nil, description: nil, is_default: true,
                on_remove_delete_file_server: nil, on_remove_delete_file_client: nil))
    }

    /// Smart-reorder the playlist's episodes by field/direction (bakes into
    /// the position order).
    func reorderPlaylist(id: Int32, field: PlaylistReorderField, direction: OrderDirection)
        async throws
    {
        try await postEmpty(
            "playlists/\(id)/reorder-by",
            body: PlaylistReorderData(field: field, direction: direction))
    }

    func addEpisode(playlistId: Int32, episodeId: Int32, position: Int32?) async throws {
        let _: EpisodePlaylistData = try await post(
            "playlists/\(playlistId)/episodes/\(episodeId)",
            body: EpisodePlaylistStoreData(position: position)
        )
    }

    func removeEpisode(playlistId: Int32, episodeId: Int32) async throws {
        try await delete("playlists/\(playlistId)/episodes/\(episodeId)")
    }

    /// Move an episode to index `to` within the playlist's position order.
    func moveEpisode(playlistId: Int32, episodeId: Int32, to: Int32) async throws {
        try await postEmpty(
            "playlists/\(playlistId)/episodes/\(episodeId)/move",
            body: EpisodePlaylistMoveData(to: to)
        )
    }
}

/// Body for `POST /playlists/{id}/move` (the wire `PlaylistMoveData` isn't in
/// the generated Swift set yet — same shape, target index only).
private struct PlaylistMoveBody: Codable {
    let to: Int32
}
