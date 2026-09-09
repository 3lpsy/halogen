import Foundation

/// The writable config-overrides allowlist — wire `ConfigOverridesData`
/// (crates/wire/src/config.rs) field-for-field: every key optional, durations
/// as whole seconds. Unset fields encode as absent (serde reads that as None).
struct ConfigOverridesData: Codable {
    var subscription_fallback_poll_interval_secs: UInt64?
    var subscription_poll_wake_interval_secs: UInt64?
    var subscription_fallback_max_episodes: UInt?
    var subscription_max_concurrent_downloads: UInt?
    var subscription_max_poll_concurrent: UInt?
    var subscription_poll_auto_download_enabled: Bool?
    var subscription_auto_playlist_add_to_start: Bool?
    var subscription_no_sync_before: String?
    var subscription_sync_on_start: Bool?
    var auth_token_expiry_minutes: UInt64?
    var episode_playback_complete_percentage: UInt16?
    var opml_file: String?

    init() {}
}

/// What `POST admin/db/import` did — wire `DbImportSummaryData`
/// (crates/wire/src/db_transfer.rs). `created_usernames` lists users the
/// import created with random passwords.
struct DbImportSummaryData: Codable {
    let users_merged: UInt32
    let users_created: UInt32
    let created_usernames: [String]
    let podcasts_merged: UInt32
    let podcasts_created: UInt32
    let subscriptions_created: UInt32
    let episodes_merged: UInt32
    let episodes_created: UInt32
    let chapters_created: UInt32
    let playbacks_upserted: UInt32
    let statuses_upserted: UInt32
    let playlists_merged: UInt32
    let playlists_created: UInt32
    let playlist_links_created: UInt32
    let auto_playlists_created: UInt32
}

/// Podcast/config/user management + OPML + raw-config reads.
extension HalogenClient {
    // MARK: - podcast management

    func updatePodcast(
        id: Int32, title: String?, description: String?, feedUrl: String?
    ) async throws {
        let _: PodcastData = try await put(
            "podcasts/\(id)",
            body: PodcastUpdateData(title: title, description: description, feed_url: feedUrl))
    }

    func deletePodcast(id: Int32) async throws {
        try await delete("podcasts/\(id)")
    }

    /// Create + link a download/poll config for a podcast (atomic).
    func createPodcastConfig(podcastId: Int32, data: PodcastConfigStoreData) async throws {
        let _: PodcastConfigData = try await post("podcasts/\(podcastId)/config", body: data)
    }

    /// One config by id — the edit-form prefill (web `get_podcast_config`):
    /// a cached podcast row can carry the config FK without the body, and
    /// editing off missing values would overwrite the real config with
    /// defaults.
    func podcastConfig(id: Int32) async throws -> PodcastConfigData {
        try await get("podcast-configs/\(id)")
    }

    /// Edit an existing config.
    func updatePodcastConfig(configId: Int32, data: PodcastConfigUpdateData) async throws {
        let _: PodcastConfigData = try await put("podcast-configs/\(configId)", body: data)
    }

    /// Unlink + delete a podcast's config.
    func deletePodcastConfig(podcastId: Int32) async throws {
        try await delete("podcasts/\(podcastId)/config")
    }

    func autoPlaylists(podcastId: Int32) async throws -> [PodcastAutoPlaylistData] {
        try await get("podcasts/\(podcastId)/auto-playlists")
    }

    /// Replace the podcast's auto-add playlist set (idempotent).
    /// `addToStart` is the per-podcast insert-position override stamped on
    /// every link: true = start, false = end, nil = server default.
    func setAutoPlaylists(podcastId: Int32, playlistIds: [Int32], addToStart: Bool?) async throws {
        var request = URLRequest(url: base.appendingPathComponent("podcasts/\(podcastId)/auto-playlists"))
        request.httpMethod = "PUT"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = try WireJSON.encoder.encode(
            RequestData<PodcastAutoPlaylistSetData, DefaultDataType>(
                data: PodcastAutoPlaylistSetData(
                    playlist_ids: playlistIds, add_to_start: addToStart),
                params: nil))
        let _: ResponseData<DefaultDataType> = try await send(request)
    }

    // MARK: - user management

    func updateUsername(userId: Int32, username: String) async throws {
        try await updateUser(userId: userId, username: username, isAdmin: nil)
    }

    /// `PUT /users/{id}` — self-edit sends `is_admin: nil` (the server forbids
    /// changing your own flag); the admin flow sends the toggle's value.
    func updateUser(userId: Int32, username: String?, isAdmin: Bool?) async throws {
        let _: UserData = try await put(
            "users/\(userId)",
            body: UserUpdateData(username: username, is_admin: isAdmin))
    }

    /// Every server user, one max-size page — admin user counts are tiny, so
    /// no lazy paging (web: AdminUsers' MAX_USERS request).
    func listUsers() async throws -> [UserData] {
        let envelope: ResponseData<[UserData]> = try await getEnvelope(
            "admin/users",
            query: [
                URLQueryItem(name: "pagination[page]", value: "0"),
                URLQueryItem(name: "pagination[size]", value: "65536"),
            ])
        guard let items = envelope.data else { throw ClientError.emptyData }
        return items
    }

    func getUser(id: Int32) async throws -> UserData {
        try await get("users/\(id)")
    }

    /// Admin: delete a user (the server refuses self-deletes independently).
    /// List + delete are admin-nested; get/update stay at /users/{id}.
    func deleteUser(id: Int32) async throws {
        try await delete("admin/users/\(id)")
    }

    /// Admin: create a user (the embedded add-account flow passes
    /// `isAdmin: true` — every embedded user is an admin by web policy).
    func createUser(username: String, password: String, isAdmin: Bool?) async throws -> UserData {
        try await post(
            "admin/users",
            body: UserStoreData(
                username: username, password: password, password_confirm: password,
                is_admin: isAdmin))
    }

    // MARK: - OPML

    func opmlExport() async throws -> String {
        let data: OpmlExportData = try await get("admin/opml/export")
        return data.opml
    }

    func opmlImport(_ opml: String) async throws -> OpmlImportResultData {
        try await post("admin/opml/import", body: OpmlImportData(opml: opml))
    }

    // MARK: - raw JSON reads (admin config surfaces — no typed DTO needed)

    /// Any envelope endpoint as a raw dictionary (config + metadata surfaces).
    func rawJSON(_ path: String) async throws -> [String: Any] {
        var request = URLRequest(url: base.appendingPathComponent(path))
        request.httpMethod = "GET"
        if let token {
            request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        }
        let (data, response) = try await LocalTransport.data(for: request)
        guard let response = response as? HTTPURLResponse else { throw URLError(.badServerResponse) }
        let status = response.statusCode
        guard (200..<300).contains(status) else { throw ClientError.http(status) }
        guard let obj = try JSONSerialization.jsonObject(with: data) as? [String: Any],
            let payload = obj["data"] as? [String: Any]
        else { throw ClientError.emptyData }
        return payload
    }

    /// The current overrides set, typed (unset keys decode as nil).
    func configOverrides() async throws -> ConfigOverridesData {
        try await get("admin/config-overrides")
    }

    /// Replace the config-overrides set wholesale with TYPED values (the
    /// server deserializes `ConfigOverridesData`; strings would 422). An
    /// empty struct clears none — use DELETE for clear-all.
    func setConfigOverrides(_ overrides: ConfigOverridesData) async throws {
        try await postEmpty("admin/config-overrides", body: overrides)
    }

    func clearConfigOverrides() async throws {
        try await delete("admin/config-overrides")
    }

    // MARK: - database transfer (raw bytes)

    func dbExport() async throws -> (Data, String) {
        var request = URLRequest(url: base.appendingPathComponent("admin/db/export"))
        request.httpMethod = "GET"
        if let token {
            request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        }
        let (data, response) = try await LocalTransport.data(for: request)
        guard let http = response as? HTTPURLResponse, (200..<300).contains(http.statusCode)
        else { throw ClientError.http((response as? HTTPURLResponse)?.statusCode ?? 0) }
        let disposition = http.value(forHTTPHeaderField: "Content-Disposition") ?? ""
        let filename =
            disposition.split(separator: "\"").dropFirst().first.map(String.init)
            ?? "halogen-export.db.gz"
        return (data, filename)
    }

    /// Upload an export (gzipped or raw SQLite); returns the typed merge
    /// summary (the embedded flow needs `created_usernames`).
    func dbImport(_ payload: Data) async throws -> DbImportSummaryData {
        var request = URLRequest(url: base.appendingPathComponent("admin/db/import"))
        request.httpMethod = "POST"
        request.setValue("application/octet-stream", forHTTPHeaderField: "Content-Type")
        request.httpBody = payload
        if let token {
            request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        }
        let (data, response) = try await LocalTransport.data(for: request)
        let status = (response as? HTTPURLResponse)?.statusCode ?? 0
        guard (200..<300).contains(status) else { throw ClientError.http(status) }
        let envelope = try WireJSON.decoder.decode(
            ResponseData<DbImportSummaryData>.self, from: data)
        if let errors = envelope.errors { throw ClientError.api(errors) }
        guard let summary = envelope.data else { throw ClientError.emptyData }
        return summary
    }

    // MARK: - shared PUT

    func put<In: Codable, Out: Codable>(_ path: String, body: In) async throws -> Out {
        var request = URLRequest(url: base.appendingPathComponent(path))
        request.httpMethod = "PUT"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = try WireJSON.encoder.encode(
            RequestData<In, DefaultDataType>(data: body, params: nil)
        )
        let envelope: ResponseData<Out> = try await send(request)
        guard let payload = envelope.data else { throw ClientError.emptyData }
        return payload
    }
}
