import Foundation
import Observation

/// App-wide core state: session lifecycle, embedded-server FFI boot, API
/// client, the account's LocalStore + Outbox, and the feature-model registry.
/// Views read `phase` and call typed helpers; nothing else touches FFI or URLs.
@MainActor
@Observable
final class HalogenCore {
    enum Phase: Equatable {
        case idle
        case starting
        /// No stored session — show the landing page (server + credentials).
        case needsAuth
        case ready
        case failed(String)
    }

    private(set) var phase: Phase = .idle
    private(set) var baseUrl: String?
    /// Who's signed in — namespaces the LocalStore. An account switch swaps
    /// this + `store` + `models` (the web app remounts on its namespace
    /// change identically).
    private(set) var account: AccountContext?
    /// The active account's local cache. `nil` only while signed out.
    private(set) var store: LocalStore?
    /// Offline mutation queue (drained whenever the server is reachable).
    private(set) var outbox: Outbox?
    private(set) var syncFailures: SyncFailures?
    /// Set when the account's LocalStore failed to open — RootView shows a
    /// persistent banner (a dead store disables persistence AND the outbox).
    private(set) var storageFailure: String?
    /// Per-account feature models. Non-nil exactly while `phase == .ready`.
    private(set) var models: Models?
    /// Whether the signed-in user is a server admin — UI gating only, like
    /// the web's `use_is_admin` (the server is the real authority). Embedded
    /// users are always admins by policy; remote is fetched after login.
    private(set) var isAdmin = false
    /// Navbar reachability dot. Probes whatever server the session points at.
    let connection = ConnectionMonitor()
    /// Set when a REMOTE session's auth died (401 the silent refresh couldn't
    /// heal): the connect page pre-fills this server + username so re-login
    /// is one password away (web: RootGuard redirects to Login with the
    /// server already known).
    private(set) var reauthHint: (serverUrl: String, username: String)?
    private var client: HalogenClient?
    /// In-flight auth-expiry recovery — collapses the burst of 401s a dead
    /// token produces into a single transition.
    private var reauthTask: Task<Void, Never>?
    /// Embedded silent re-login is bounded (web: 3 attempts) so a server
    /// that keeps rejecting fresh tokens degrades to signed-out, not a loop.
    private var embeddedReloginAttempts = 0
    /// The periodic drain+pull loop (web: PULL_INTERVAL_SECS) — lives for the
    /// session, torn down with it.
    private var pullTask: Task<Void, Never>?

    /// Whether mutations should queue rather than go direct (the web's
    /// `is_offline` gate for the direct-vs-outbox form submit paths).
    var isOffline: Bool {
        connection.manualOffline || connection.status == .offline
    }

    // MARK: - lifecycle

    /// App start: resume the stored session, or land on auth. Local-first: a
    /// stored REMOTE session resumes into `.ready` even if the server is
    /// unreachable — cached lists render and the dot shows offline.
    func boot() async {
        guard phase == .idle else { return }
        phase = .starting
        initCore()
        guard let session = SessionStore.load().active else {
            phase = .needsAuth
            return
        }
        await resume(session)
    }

    /// Resume any stored session (boot + account switch share this).
    /// Local-first: remote sessions go straight to `.ready` even offline.
    private func resume(_ session: Session) async {
        switch session.kind {
        case .embedded:
            do {
                try await startEmbedded(as: session)
            } catch {
                DeviceLog.warn("embedded resume failed — \(error)")
                phase = .failed(FriendlyError.message(error))
            }
        case .remote:
            guard let serverUrl = session.serverUrl else {
                phase = .needsAuth
                return
            }
            let client = HalogenClient(baseUrl: serverUrl, token: session.token)
            await finish(
                client: client,
                kind: .remote(serverUrl: serverUrl),
                serverUrl: serverUrl,
                username: session.username,
                jwt: session.token
            )
        }
    }

    /// Settings → Accounts: activate a different saved session.
    func switchAccount(_ session: Session) async {
        SessionStore.switchTo(session.id)
        teardownSession()
        phase = .starting
        await resume(session)
    }

    /// Settings → Accounts: land on the connect page WITHOUT dropping saved
    /// sessions (the new login upserts into the registry).
    func beginAddAccount() {
        addAccountReturnSession = SessionStore.load().active
        teardownSession()
        phase = .needsAuth
    }

    /// The session to return to when the add-account landing is cancelled.
    /// Set ONLY by `beginAddAccount` — an expired session offers no way back,
    /// so its `.needsAuth` landing shows no back button.
    private(set) var addAccountReturnSession: Session?

    /// Back out of add-account: resume the session the user came from.
    func cancelAddAccount() async {
        guard let back = addAccountReturnSession else { return }
        addAccountReturnSession = nil
        await switchAccount(back)
    }

    enum EmbeddedUserError: Error, CustomStringConvertible {
        case tooShort
        case exists(String)

        var description: String {
            switch self {
            case .tooShort:
                return "Username must be at least 3 characters"
            case .exists(let name):
                return "'\(name)' already exists — switch to it instead"
            }
        }
    }

    /// Create + switch to a new EMBEDDED user (app-generated password). Web
    /// create_embedded_user parity: lowercase + 3-char minimum + duplicate
    /// guard, admin by policy, secret persisted only after the create succeeds.
    func addEmbeddedUser(username raw: String) async throws {
        guard let baseUrl, case .embedded = account?.kind else {
            throw ConnectError.invalidUrl
        }
        let username = raw.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        guard username.count >= 3 else { throw EmbeddedUserError.tooShort }
        if SessionStore.load().sessions.contains(where: {
            $0.kind == .embedded && $0.username == username
        }) {
            throw EmbeddedUserError.exists(username)
        }

        let password: String
        if let stored = EmbeddedSecrets.password(for: username) {
            // Known credential without a registered session (the account was
            // removed): reconnect with it — a fresh create would 409.
            password = stored
        } else {
            password = UUID().uuidString
            // Every embedded user is an admin of the on-device server (web:
            // the add-user form locks the toggle on). Create server-side
            // FIRST; persist the secret only on success.
            _ = try await requireClient().createUser(
                username: username, password: password, isAdmin: true)
            EmbeddedSecrets.remember(username: username, password: password)
        }
        let client = HalogenClient(baseUrl: baseUrl)
        let jwt = try await client.login(username: username, password: password)
        let session = Session(
            kind: .embedded, serverUrl: nil, username: username, token: jwt,
            password: password)
        SessionStore.upsertActive(session)
        teardownSession()
        phase = .starting
        await resume(session)
    }

    /// Recreate the per-account models over the same session (post-purge).
    func remountModels() {
        models = Models(core: self)
        Task { await models?.nav.load() }
        Task { await models?.swipes.load() }
        Task { await models?.prefs.load() }
        Task { await models?.device.load() }
        Task { await models?.playbacks.load() }
        Task { await models?.podcasts.seed() }
        Task { await models?.playlists.seed() }
        // Resolve the queue at mount (cache-first) so "Add to Queue" works
        // from any tab — including offline cold starts — before the Queue
        // screen has ever been opened (web: hydrate → recompute_queue).
        Task { await models?.queue.load() }
    }

    /// Local Data: destroy the embedded library (stops the in-process server
    /// and deletes db/media/secrets). Embedded sessions are removed from the
    /// registry; when the ACTIVE account was embedded this signs out to the
    /// next account or the landing page.
    func destroyEmbeddedServer() async throws {
        let root = try Self.embeddedRoot()
        try await destroyEmbedded(dataRoot: root.path)
        try? FileManager.default.removeItem(at: root)
        // The server-side users are gone — their silent-login credentials too.
        EmbeddedSecrets.clear()

        var registry = SessionStore.load()
        let activeWasEmbedded =
            registry.active.map { $0.kind == .embedded } ?? false
        // The embedded sessions' CLIENT caches must die with the library — a
        // re-created embedded user with the same id would inherit stale
        // lists/prefs from the old world otherwise.
        if let support = try? FileManager.default.url(
            for: .applicationSupportDirectory, in: .userDomainMask,
            appropriateFor: nil, create: false)
        {
            let clientRoot = support.appendingPathComponent(
                "halogen-client", isDirectory: true)
            for session in registry.sessions where session.kind == .embedded {
                // Embedded namespaces are "e{userId}" (AccountContext) — the
                // user id rides the session token's sub claim.
                guard let sub = AccountContext.jwtSub(session.token) else { continue }
                try? FileManager.default.removeItem(
                    at: clientRoot.appendingPathComponent("e\(sub)"))
            }
        }
        registry.sessions.removeAll { $0.kind == .embedded }
        if activeWasEmbedded {
            registry.activeId = registry.sessions.last?.id
        }
        SessionStore.save(registry)

        if activeWasEmbedded {
            teardownSession()
            if let next = registry.active {
                phase = .starting
                await resume(next)
            } else {
                phase = .needsAuth
            }
        }
    }

    func renameActiveAccount(to username: String) {
        SessionStore.renameActive(to: username)
        if let account {
            self.account = AccountContext(
                kind: account.kind, userId: account.userId, username: username)
        }
    }

    /// Landing page: connect to a remote server — reachability probe, then
    /// login (the web's server-setup → login flow, on one page). Throws so
    /// the form can show the failure inline.
    func connectRemote(serverUrl: String, username: String, password: String) async throws {
        let normalized = Self.normalizeServerUrl(serverUrl)
        guard let parsed = URL(string: normalized),
            ["http", "https"].contains(parsed.scheme ?? "")
        else {
            throw ConnectError.invalidUrl
        }
        let client = HalogenClient(baseUrl: normalized)
        try await client.health()
        let jwt = try await client.login(username: username, password: password)
        SessionStore.upsertActive(
            Session(kind: .remote, serverUrl: normalized, username: username, token: jwt)
        )
        await finish(
            client: client,
            kind: .remote(serverUrl: normalized),
            serverUrl: normalized,
            username: username,
            jwt: jwt
        )
    }

    /// Landing page: use this device's own library (the embedded server).
    func useLocalLibrary() async {
        phase = .starting
        do {
            try await startEmbedded(as: nil)
        } catch {
            DeviceLog.warn("embedded start failed — \(error)")
            phase = .failed(FriendlyError.message(error))
        }
    }

    /// Boot-failure screen: try the stored session again (a transient
    /// embedded-server hiccup shouldn't strand the user on an error page).
    func retryBoot() async {
        guard case .failed = phase else { return }
        phase = .idle
        await boot()
    }

    /// A 401 the token refresh couldn't heal — auth is dead. Embedded accounts
    /// re-login silently with on-disk credentials; remote accounts return to
    /// the connect page with session and cache KEPT, so re-login resumes the
    /// same account namespace (web: RootGuard routes to a prefilled Login).
    private func handleAuthExpired() {
        guard phase == .ready, reauthTask == nil else { return }
        reauthTask = Task { [weak self] in
            await self?.recoverExpiredAuth()
            self?.reauthTask = nil
        }
    }

    private func recoverExpiredAuth() async {
        guard phase == .ready, let account else { return }
        switch account.kind {
        case .embedded:
            embeddedReloginAttempts += 1
            if embeddedReloginAttempts <= 3 {
                var password = EmbeddedSecrets.password(for: account.username)
                if password == nil,
                    let root = try? Self.embeddedRoot(),
                    let creds = try? await recoverUser(
                        dataRoot: root.path, username: account.username)
                {
                    // No stored credential for this username (pre-gating
                    // server-side rename): rotate the row in the DB and adopt
                    // the fresh secret (web: recover_user on the 401 path).
                    EmbeddedSecrets.remember(
                        username: creds.username, password: creds.password)
                    password = creds.password
                }
                if let password,
                    let jwt = try? await client?.login(
                        username: account.username, password: password)
                {
                    // login() already refreshed the shared TokenBox; mirror it
                    // into the saved session + art loader like a refresh would.
                    client?.tokenBox.onRefresh?(jwt)
                    DeviceLog.info("auth: embedded session re-authenticated silently")
                    return
                }
            }
            // Out of budget (or the secret is gone): degrade to the landing
            // page. The session stays saved — the next boot retries the
            // full embedded resume path.
            DeviceLog.warn("auth: embedded silent re-login failed")
            teardownSession()
            phase = .needsAuth
        case .remote(let serverUrl):
            DeviceLog.warn("auth: remote session expired; returning to connect page")
            let username = account.username
            teardownSession()
            reauthHint = (serverUrl: serverUrl, username: username)
            phase = .needsAuth
            ToastCenter.shared.error("Session expired — sign in again")
        }
    }

    /// Remove the active account. Another saved account takes over if one
    /// exists; otherwise back to the landing page. Cached data stays on disk
    /// (its namespace makes it inert until the same account signs in again).
    func signOut() {
        let registry = SessionStore.load()
        let next = registry.activeId.flatMap { SessionStore.remove($0) }
        teardownSession()
        if let next {
            phase = .starting
            Task { await resume(next) }
        } else {
            phase = .needsAuth
        }
    }

    private func teardownSession() {
        pullTask?.cancel()
        pullTask = nil
        reauthTask?.cancel()
        reauthTask = nil
        reauthHint = nil
        client = nil
        account = nil
        store = nil
        outbox = nil
        syncFailures = nil
        storageFailure = nil
        models = nil
        baseUrl = nil
        isAdmin = false
        connection.stop()
    }

    // MARK: - typed fetches (thin passthroughs so models never hold a client)

    func podcasts(page: Int = 0) async throws -> HalogenClient.PageOf<PodcastData> {
        try await requireClient().podcasts(page: page)
    }

    func episodes(
        podcastId: Int32, extra: [URLQueryItem] = [], page: Int = 0
    ) async throws -> HalogenClient.PageOf<EpisodeData> {
        try await requireClient().episodes(podcastId: podcastId, extra: extra, page: page)
    }

    /// Raw facet variant (Downloads page and friends supply their own params).
    func latestEpisodesRaw(
        extra: [URLQueryItem],
        page: Int = 0,
        pageSize: Int = 20
    ) async throws -> HalogenClient.PageOf<EpisodeData> {
        try await requireClient().latestEpisodes(filter: extra, page: page, pageSize: pageSize)
    }

    func playlistsList() async throws -> [PlaylistData] {
        try await requireClient().playlists()
    }

    func defaultPlaylist() async throws -> PlaylistData? {
        try await requireClient().defaultPlaylist()
    }

    func playlistEpisodes(playlistId: Int32) async throws -> [EpisodeData] {
        try await requireClient().playlistEpisodes(playlistId: playlistId)
    }

    func createPlaylist(
        name: String, description: String? = nil, isDefault: Bool,
        deleteServerFile: Bool = false, deleteClientFile: Bool = false
    ) async throws -> PlaylistData {
        try await requireClient().createPlaylist(
            name: name, description: description, isDefault: isDefault,
            deleteServerFile: deleteServerFile, deleteClientFile: deleteClientFile)
    }

    func deletePlaylist(id: Int32) async throws {
        try await requireClient().deletePlaylist(id: id)
    }

    /// Full playlist edit (the web form's field set; nil = leave unchanged).
    func updatePlaylist(
        id: Int32, name: String?, description: String?, isDefault: Bool?,
        deleteServerFile: Bool?, deleteClientFile: Bool?
    ) async throws {
        try await requireClient().updatePlaylist(
            id: id, name: name, isDefault: isDefault, description: description,
            deleteServerFile: deleteServerFile, deleteClientFile: deleteClientFile)
        if isDefault == true { await models?.queue.refresh() }
    }

    func makeQueuePlaylist(id: Int32) async throws {
        try await requireClient().makeQueuePlaylist(id: id)
        await models?.queue.refresh()
    }

    func reorderPlaylist(id: Int32, field: PlaylistReorderField, direction: OrderDirection)
        async throws
    {
        try await requireClient().reorderPlaylist(id: id, field: field, direction: direction)
    }

    func episodeDetail(id: Int32) async throws -> EpisodeData {
        try await requireClient().episode(id: id)
    }

    /// Overlay-wins played-status read for local list filtering — the chips
    /// must flip with a local mark-played, not wait for the next pull (web:
    /// the cached facet is kept in lock-step with the overlay).
    func overlayStatus(_ episode: EpisodeData) -> PlaybackStatus {
        models?.playbacks.status(for: episode) ?? episode.playback_status ?? .unplayed
    }

    /// Deep-link fetch-throughs (web get_podcast/get_playlist): resolve a
    /// by-id navigation target the local pools don't hold yet.
    func podcast(id: Int32) async throws -> PodcastData {
        try await requireClient().podcast(id: id)
    }

    func playlist(id: Int32) async throws -> PlaylistData {
        try await requireClient().playlist(id: id)
    }

    /// One page of playback rows, updated_at desc (the History source).
    func playbacksPage(page: Int, pageSize: Int = 20) async throws
        -> HalogenClient.PageOf<PlaybackData>
    {
        try await requireClient().playbacks(page: page, pageSize: pageSize)
    }

    func discoverSearch(query: String, providers: [DiscoverProvider]? = nil) async throws
        -> DiscoverSearchData
    {
        try await requireClient().discoverSearch(query: query, providers: providers)
    }

    func discoverProviders() async throws -> DiscoverProvidersData {
        try await requireClient().discoverProviders()
    }

    func subscribePodcast(title: String, feedUrl: String, description: String?) async throws
        -> PodcastData
    {
        try await requireClient().createPodcast(
            title: title, feedUrl: feedUrl, description: description)
    }

    /// The streaming audio URL for AVPlayer (`nil` while signed out).
    func audioURL(episodeId: Int32) -> URL? {
        try? requireClient().audioURL(episodeId: episodeId)
    }

    /// The raw API JWT — AVURLAsset needs it as a literal header (it can't go
    /// through the client's request path).
    var apiToken: String? {
        client?.token
    }

    /// The navbar's manual offline toggle (web parity): pause probing +
    /// outbox drains; flipping back online probes and drains immediately.
    func setManualOffline(_ offline: Bool) {
        connection.setManualOffline(offline)
        Task { await outbox?.setSuspended(offline) }
        if !offline {
            Task { await outbox?.drain() }
        }
        // Persisted: a relaunch must come back in the chosen mode (web
        // ClientConfig.manual_offline). Embedded never persists true — the
        // on-device server is always reachable.
        let persisted = offline && !isEmbeddedAccount
        Task { [store] in await store?.save(persisted, key: CacheKey.manualOffline) }
        DeviceLog.info(offline ? "went manually offline" : "back online (manual)")
    }

    /// Boot-time re-apply of the persisted manual-offline choice.
    func restoreManualOffline() async {
        guard !isEmbeddedAccount,
            await store?.load(Bool.self, key: CacheKey.manualOffline) == true
        else { return }
        connection.setManualOffline(true)
        await outbox?.setSuspended(true)
        DeviceLog.info("restored manual offline from last session")
    }

    var isEmbeddedAccount: Bool {
        if case .embedded = account?.kind { return true }
        return false
    }

    /// The strategy playback actually uses: embedded accounts always stream
    /// (their media already lives on this device inside the server).
    var effectivePlaybackStrategy: ClientPrefs.PlaybackStrategy {
        isEmbeddedAccount
            ? .streamOnly : (models?.prefs.prefs.playbackStrategy ?? .downloadOnly)
    }

    func rawJSON(_ path: String) async throws -> [String: Any] {
        try await requireClient().rawJSON(path)
    }

    /// Admin: create a server user with an explicit password (POST
    /// /admin/users — the embedded add-user flow reuses the same endpoint
    /// with a generated secret).
    func createUser(username: String, password: String, isAdmin: Bool) async throws {
        _ = try await requireClient().createUser(
            username: username, password: password, isAdmin: isAdmin)
    }

    func configOverrides() async throws -> ConfigOverridesData {
        try await requireClient().configOverrides()
    }

    func setConfigOverrides(_ overrides: ConfigOverridesData) async throws {
        try await requireClient().setConfigOverrides(overrides)
    }

    func clearConfigOverrides() async throws {
        try await requireClient().clearConfigOverrides()
    }

    func updatePodcast(id: Int32, title: String?, description: String?, feedUrl: String?)
        async throws
    {
        try await requireClient().updatePodcast(
            id: id, title: title, description: description, feedUrl: feedUrl)
    }

    func deletePodcast(id: Int32) async throws {
        try await requireClient().deletePodcast(id: id)
    }

    /// Unsubscribe, offline-capable (web: durable `Unsubscribe` op):
    /// tombstone + optimistic removal, local-cache cascade, queued delete,
    /// then a reconcile refresh (which filters against the tombstone, so it
    /// is safe even while the delete is still queued).
    func unsubscribePodcast(id: Int32) async {
        await models?.podcasts.tombstone(id: id)
        await purgePodcastLocalData(podcastId: id)
        await outbox?.enqueue(.unsubscribe(podcastId: id))
        await models?.podcasts.refresh()
    }

    /// The web's `delete_podcast` cascade, native (ui-svc-store store.rs):
    /// prune every local trace of an unsubscribed podcast so no snapshot can
    /// rehydrate it. Episode ids resolve from the podcast's episode snapshot
    /// FIRST (playbacks are keyed by episode id) — same order as the web.
    private func purgePodcastLocalData(podcastId: Int32) async {
        guard let store else { return }
        let episodes =
            await store.load([EpisodeData].self, key: CacheKey.podcastEpisodes(podcastId)) ?? []
        for episode in episodes {
            models?.playbacks.purge(episodeId: episode.id)
            await store.remove(key: CacheKey.episode(episode.id))
            await store.remove(key: CacheKey.metadata("episodes/\(episode.id)"))
        }
        await store.remove(key: CacheKey.podcastEpisodes(podcastId))
        await store.remove(key: CacheKey.autoPlaylists(podcastId))
        await store.remove(key: CacheKey.metadata("podcasts/\(podcastId)"))
        // Cross-podcast list snapshots (latest-*/downloads-*): their
        // prefix-merge would otherwise keep this podcast's rows as tail
        // forever (server truth only ever rewrites page 0).
        for key in await store.listKeys()
        where (key.hasPrefix("latest-") || key.hasPrefix("downloads-"))
            && key != CacheKey.latestScrollAnchor
        {
            guard var rows = await store.load([EpisodeData].self, key: key) else { continue }
            let before = rows.count
            rows.removeAll { $0.podcast_id == podcastId }
            if rows.count != before { await store.save(rows, key: key) }
        }
        models?.latest.removePodcastLocally(podcastId)
        models?.downloads.removePodcastLocally(podcastId)
    }

    func createPodcastConfig(podcastId: Int32, data: PodcastConfigStoreData) async throws {
        try await requireClient().createPodcastConfig(podcastId: podcastId, data: data)
    }

    func podcastConfig(id: Int32) async throws -> PodcastConfigData {
        try await requireClient().podcastConfig(id: id)
    }

    func updatePodcastConfig(configId: Int32, data: PodcastConfigUpdateData) async throws {
        try await requireClient().updatePodcastConfig(configId: configId, data: data)
    }

    func deletePodcastConfig(podcastId: Int32) async throws {
        try await requireClient().deletePodcastConfig(podcastId: podcastId)
    }

    func autoPlaylists(podcastId: Int32) async throws -> [PodcastAutoPlaylistData] {
        try await requireClient().autoPlaylists(podcastId: podcastId)
    }

    func setAutoPlaylists(podcastId: Int32, playlistIds: [Int32], addToStart: Bool?) async throws {
        try await requireClient().setAutoPlaylists(
            podcastId: podcastId, playlistIds: playlistIds, addToStart: addToStart)
    }

    func updateUsername(userId: Int32, username: String) async throws {
        try await requireClient().updateUsername(userId: userId, username: username)
    }

    // MARK: - admin user management (web: AdminUsers / AdminUserEdit)

    func listUsers() async throws -> [UserData] {
        try await requireClient().listUsers()
    }

    func updateUser(userId: Int32, username: String?, isAdmin: Bool?) async throws {
        try await requireClient().updateUser(userId: userId, username: username, isAdmin: isAdmin)
    }

    func deleteUser(id: Int32) async throws {
        try await requireClient().deleteUser(id: id)
    }

    func dbExport() async throws -> (Data, String) {
        try await requireClient().dbExport()
    }

    func dbImport(_ payload: Data) async throws -> DbImportSummaryData {
        try await requireClient().dbImport(payload)
    }

    /// After an embedded DB import: users the import created got RANDOM
    /// passwords the app doesn't know — rotate each into EmbeddedSecrets so
    /// switching to them silently just works (web: align_imported_users).
    /// Best-effort per user; returns the usernames that couldn't be aligned.
    func alignImportedUsers(_ usernames: [String]) async -> [String] {
        guard isEmbeddedAccount, let root = try? Self.embeddedRoot() else {
            return usernames
        }
        var failed: [String] = []
        for username in usernames {
            do {
                let creds = try await recoverUser(dataRoot: root.path, username: username)
                EmbeddedSecrets.remember(username: creds.username, password: creds.password)
            } catch {
                DeviceLog.warn(
                    "db import: couldn't align '\(username)' for silent login: \(error)")
                failed.append(username)
            }
        }
        return failed
    }

    func opmlExport() async throws -> String {
        try await requireClient().opmlExport()
    }

    func opmlImport(_ opml: String) async throws -> OpmlImportResultData {
        try await requireClient().opmlImport(opml)
    }

    func pollJobs() async throws -> [PollJobData] {
        try await requireClient().pollJobs()
    }

    func startPollJob() async throws -> UInt64 {
        try await requireClient().startPollJob()
    }

    func serverLogs() async throws -> ServerLogsData {
        try await requireClient().serverLogs()
    }

    func serverErrors() async throws -> ServerErrorsData {
        try await requireClient().serverErrors()
    }

    func changePassword(current: String, new: String) async throws {
        try await requireClient().changePassword(current: current, new: new)
    }

    func triggerDownload(episodeId: Int32) async throws {
        try await requireClient().triggerDownload(episodeId: episodeId)
    }

    func removeServerDownload(episodeId: Int32) async throws {
        try await requireClient().removeServerDownload(episodeId: episodeId)
    }

    func downloadProgress(episodeId: Int32) async throws -> DownloadProgressData? {
        try await requireClient().downloadProgress(episodeId: episodeId)
    }

    // MARK: - media URLs (the server art cache; ArtLoader adds the bearer)

    func podcastArtURL(_ podcast: PodcastData, small: Bool = true) -> URL? {
        artURL(kind: "podcasts", id: podcast.id, small: small)
    }

    func episodeArtURL(_ episode: EpisodeData, small: Bool = true) -> URL? {
        artURL(kind: "episodes", id: episode.id, small: small)
    }

    // MARK: - internals

    enum ConnectError: Error, CustomStringConvertible {
        case invalidUrl

        var description: String { "Server URL must start with http:// or https://" }
    }

    /// Which self-heal a failed embedded login triggers (web `Recovery`):
    /// seeded-admin recovers via `recover_admin` (also ADOPTS a renamed admin
    /// row); a specific user rotates via `recover_user`, falling back to admin
    /// adoption when the row under that name is gone.
    private enum EmbeddedRecovery {
        case admin
        case user(String)
    }

    private func recoverEmbeddedCredentials(
        root: String, _ recovery: EmbeddedRecovery
    ) async throws -> EmbeddedCredentials {
        switch recovery {
        case .admin:
            return try await recoverAdmin(dataRoot: root)
        case .user(let username):
            do {
                return try await recoverUser(dataRoot: root, username: username)
            } catch {
                return try await recoverAdmin(dataRoot: root)
            }
        }
    }

    /// The embedded server's on-disk library (Application Support container).
    private static func embeddedRoot() throws -> URL {
        let support = try FileManager.default.url(
            for: .applicationSupportDirectory,
            in: .userDomainMask,
            appropriateFor: nil,
            create: true
        )
        return support.appendingPathComponent("halogen-server", isDirectory: true)
    }

    /// Boot (or reuse) the in-process server, then silent-login (provisioned
    /// admin, or a stored embedded user's app-managed credentials). Rejected
    /// credentials self-heal by rotating the row's password in the DB — an
    /// embedded account never lands on a password prompt.
    private func startEmbedded(as session: Session?) async throws {
        let root = try Self.embeddedRoot()
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)

        let url = try startServer(dataRoot: root.path)
        _ = try await waitReady()
        // The embedded server's tracing is only visible through the core's
        // device-log ring — start folding it into the native Device Logs.
        DeviceLog.shared.startCorePump()

        let recovery: EmbeddedRecovery =
            session.map { .user($0.username) } ?? .admin
        var (username, password): (String, String)
        if let session, let stored = session.password {
            (username, password) = (session.username, stored)
        } else if let session, let stored = EmbeddedSecrets.password(for: session.username) {
            // The app-side secrets mirror (survives session removal) — the
            // web's per-username secrets.json lookup.
            (username, password) = (session.username, stored)
        } else if session != nil {
            // A stored session with NO credential anywhere — e.g. the account
            // was renamed server-side before the rename gating landed, so the
            // secrets track the old name. Rotate the row in the DB instead of
            // stranding the user on the landing page.
            let creds = try await recoverEmbeddedCredentials(root: root.path, recovery)
            (username, password) = (creds.username, creds.password)
        } else {
            let creds = try credentials(dataRoot: root.path)
            (username, password) = (creds.username, creds.password)
        }
        let client = HalogenClient(baseUrl: url)
        let jwt: String
        do {
            jwt = try await client.login(username: username, password: password)
        } catch {
            // Credential drift (stale secrets, out-of-band change): rotate
            // the password in the DB and retry once (web: login_with).
            DeviceLog.warn("auth: embedded credentials rejected — running recovery")
            let fresh = try await recoverEmbeddedCredentials(root: root.path, recovery)
            (username, password) = (fresh.username, fresh.password)
            jwt = try await client.login(username: username, password: password)
        }
        // Mirror the working credential into the app-side secrets store so a
        // later session removal never strands this account (web parity:
        // sign-out is always recoverable, nothing is deleted).
        EmbeddedSecrets.remember(username: username, password: password)
        var saved =
            session
            ?? Session(kind: .embedded, serverUrl: nil, username: username, token: jwt)
        // Recovery may have ADOPTED a renamed admin row — the session must
        // track the username that actually signed in.
        saved.username = username
        saved.password = password
        saved.token = jwt
        SessionStore.upsertActive(saved)
        await finish(
            client: client,
            kind: .embedded,
            serverUrl: url,
            username: username,
            jwt: jwt
        )
    }

    private func finish(
        client: HalogenClient,
        kind: AccountContext.Kind,
        serverUrl: String,
        username: String,
        jwt: String
    ) async {
        self.client = client
        self.baseUrl = serverUrl
        let account = AccountContext.from(kind: kind, username: username, jwt: jwt)
        self.account = account
        // Admin gating (web: ClientConfig.is_admin): embedded users are
        // always admins by policy; remote resolves from the current user
        // after login. Best-effort and UI-only — the server is the authority.
        switch kind {
        case .embedded:
            isAdmin = true
        case .remote:
            isAdmin = false
            if let account {
                Task { [weak self] in
                    let me = try? await client.getUser(id: account.userId)
                    self?.isAdmin = me?.is_admin ?? false
                }
            }
        }
        // "anon" only if the JWT is somehow malformed — never expected, but a
        // working un-namespaced cache beats a crash (web does the same).
        let store: LocalStore?
        do {
            store = try LocalStore(namespace: account?.namespace ?? "anon")
            storageFailure = nil
        } catch {
            // A dead store silently no-ops EVERY optimistic mutation (nil
            // outbox) — say so loudly and persistently (RootView banner).
            store = nil
            storageFailure =
                "Local storage is unavailable — changes made here won't be saved or synced."
            DeviceLog.error("localstore: init failed — \(error)")
        }
        self.store = store
        if let store {
            let failures = SyncFailures(store: store)
            self.syncFailures = failures
            self.outbox = await Outbox(
                store: store,
                perform: { [weak self] op in try await self?.execute(op) },
                onDeadLetter: { [weak self] op, error in
                    await failures.record(op: op, error: error)
                    await self?.healAfterDeadLetter(op)
                }
            )
        }
        ArtLoader.shared.configure(token: client.token, namespace: account?.namespace)
        client.tokenBox.onRefresh = { fresh in
            // Keep the persisted session + art loader on the fresh token.
            var registry = SessionStore.load()
            if let idx = registry.sessions.firstIndex(where: { $0.id == registry.activeId }) {
                registry.sessions[idx].token = fresh
                SessionStore.save(registry)
            }
            ArtLoader.shared.configure(token: fresh)
        }
        client.tokenBox.onAuthExpired = { [weak self, box = client.tokenBox] in
            Task { @MainActor in
                // Ignore stale sessions: an in-flight request from before an
                // account switch must not kick the NEW session to the
                // connect page.
                guard let self, self.client?.tokenBox === box else { return }
                self.handleAuthExpired()
            }
        }
        reauthHint = nil
        embeddedReloginAttempts = 0
        connection.onOnline = { [weak self] in
            Task { await self?.resyncAfterReconnect() }
        }
        connection.start(baseUrl: serverUrl)
        self.models = Models(core: self)
        Task { await models?.nav.load() }
        Task { await models?.swipes.load() }
        Task { await models?.prefs.load() }
        Task {
            await models?.device.load()
            // Staged partials continue without a manual tap (web:
            // resume_partial_downloads).
            models?.device.resumePartials()
        }
        Task { await models?.playbacks.load() }
        // Pool hydration (web: hydrate_from_store fills every pool at boot):
        // podcasts + playlists seed cache-only so by-id navigation and the
        // row menus' playlist toggles work from any tab, offline included —
        // no network until the owning screen revalidates.
        Task { await models?.podcasts.seed() }
        Task { await models?.playlists.seed() }
        // Resolve the queue at boot (cache-first) so "Add to Queue" works
        // from any tab — including offline cold starts — before the Queue
        // screen has ever been opened (web: hydrate → recompute_queue +
        // EnsureDefaultPlaylist).
        Task { await models?.queue.load() }
        // Restore BEFORE the first drain, or the drain races the suspension.
        Task {
            await restoreManualOffline()
            await outbox?.drain()
        }
        startPeriodicPull()
        // Any session reaching ready invalidates a pending add-account
        // return target — a later expiry must not offer a stale "back".
        addAccountReturnSession = nil
        phase = .ready
    }

    /// A dropped op leaves optimistic state lying (a green "Subscribed" row,
    /// a hidden podcast, a reordered queue): reconcile promptly, while the
    /// dead-letter toast explains why — not on some later unrelated refresh.
    private func healAfterDeadLetter(_ op: OutboxOp) async {
        guard let models else { return }
        switch op.kind {
        case .subscribe(let feedUrl, _, _):
            models.discover.noteSubscribeFailed(feedUrl: feedUrl)
            if models.podcasts.loaded { await models.podcasts.refresh() }
        case .unsubscribe:
            if models.podcasts.loaded { await models.podcasts.refresh() }
        case .addToPlaylist, .removeFromPlaylist, .moveInPlaylist, .reorderPlaylist:
            await models.queue.refresh()
            if models.playlists.loaded { await models.playlists.refresh() }
        default:
            break
        }
    }

    /// The web worker's 60s tick: drain queued ops, then revalidate the queue
    /// (its GET /playlists/default is the web's whole periodic pull too — the
    /// open screens revalidate through their own lifecycles).
    private func startPeriodicPull() {
        pullTask?.cancel()
        pullTask = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: .seconds(60))
                guard !Task.isCancelled, let self else { return }
                guard self.connection.status == .online else { continue }
                await self.outbox?.drain()
                await self.models?.queue.refresh()
            }
        }
    }

    /// Foreground return: probe reachability AND drain-then-pull immediately —
    /// the 60s tick and the offline→online transition both miss "came back to
    /// a backgrounded app whose network never changed". Drain BEFORE pulling
    /// (web order: the server must reflect optimistic ops before reads).
    func foregroundSync() async {
        await connection.probe()
        guard phase == .ready else { return }
        await outbox?.drain()
        await models?.queue.refresh()
    }

    private func resyncAfterReconnect() async {
        await outbox?.drain()
        // Admin gating resolved over a dead connection sticks false for the
        // whole session (the nav section vanishes) — re-resolve when healthy.
        if case .remote = account?.kind, !isAdmin, let account, let client {
            if let me = try? await client.getUser(id: account.userId) {
                isAdmin = me.is_admin
            }
        }
        guard let models else { return }
        if models.queue.loaded { await models.queue.refresh() }
        if models.playlists.loaded { await models.playlists.refresh() }
        async let latest: Void = models.latest.loaded ? models.latest.refresh() : ()
        async let podcasts: Void = models.podcasts.loaded ? models.podcasts.refresh() : ()
        async let history: Void = models.history.loaded ? models.history.refresh() : ()
        async let downloads: Void = models.downloads.loaded ? models.downloads.refresh() : ()
        _ = await (latest, podcasts, history, downloads)
    }

    /// The Outbox's executor — one queued op against the API.
    private func execute(_ op: OutboxOp) async throws {
        let client = try requireClient()
        switch op.kind {
        case .addToPlaylist(let playlistId, let episodeId, let position):
            try await client.addEpisode(
                playlistId: playlistId, episodeId: episodeId, position: position)
        case .removeFromPlaylist(let playlistId, let episodeId):
            try await client.removeEpisode(playlistId: playlistId, episodeId: episodeId)
        case .moveInPlaylist(let playlistId, let episodeId, let to):
            try await client.moveEpisode(playlistId: playlistId, episodeId: episodeId, to: to)
        case .setCursor(let episodeId, let cursor):
            try await client.upsertPlayback(episodeId: episodeId, cursor: cursor, completed: false)
        case .setPlayed(let episodeId, let played):
            // Same shape the web outbox sends: cursor 0 + the completed flag.
            try await client.upsertPlayback(episodeId: episodeId, cursor: 0, completed: played)
        case .reorderPlaylist(let playlistId, let field, let direction):
            try await client.reorderPlaylist(id: playlistId, field: field, direction: direction)
        case .updatePlaylist(
            let playlistId, let name, let isDefault,
            let description, let deleteServerFile, let deleteClientFile):
            try await client.updatePlaylist(
                id: playlistId, name: name, isDefault: isDefault, description: description,
                deleteServerFile: deleteServerFile, deleteClientFile: deleteClientFile)
        case .movePlaylist(let playlistId, let to):
            try await client.movePlaylist(id: playlistId, to: to)
        case .subscribe(let feedUrl, let title, let description):
            // The DTO requires a 1-256 char title; fall back to the feed URL —
            // the RSS ingest heals it to the channel title (web parity).
            // Clamp the description too: an oversize directory blurb would
            // 422 and permanently drop the queued subscription.
            let trimmed = title?.trimmingCharacters(in: .whitespaces) ?? ""
            _ = try await client.createPodcast(
                title: String((trimmed.isEmpty ? feedUrl : trimmed).prefix(256)),
                feedUrl: feedUrl,
                description: description.map { String($0.prefix(4096)) })
        case .unsubscribe(let podcastId):
            try await client.deletePodcast(id: podcastId)
        case .triggerDownload(let episodeId):
            try await client.triggerDownload(episodeId: episodeId)
        case .removeServerDownload(let episodeId):
            try await client.removeServerDownload(episodeId: episodeId)
        case .updatePodcastConfig(let configId, let data):
            try await client.updatePodcastConfig(configId: configId, data: data)
        case .removePodcastConfig(let podcastId):
            try await client.deletePodcastConfig(podcastId: podcastId)
        case .setAutoPlaylists(let podcastId, let playlistIds, let addToStart):
            try await client.setAutoPlaylists(
                podcastId: podcastId, playlistIds: playlistIds, addToStart: addToStart)
        }
    }

    private func artURL(kind: String, id: Int32, small: Bool) -> URL? {
        guard let baseUrl else { return nil }
        let suffix = small ? "/art/small" : "/art"
        return URL(string: "\(baseUrl)/api/v1/\(kind)/\(id)\(suffix)")
    }

    private static func normalizeServerUrl(_ raw: String) -> String {
        var s = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        while s.hasSuffix("/") { s.removeLast() }
        return s
    }

    private func requireClient() throws -> HalogenClient {
        guard let client else {
            throw HalogenClient.ClientError.signedOut
        }
        // Manual offline is a HARD gate on every API read/write (not just the
        // outbox — lists must stop paginating too). Embedded accounts are
        // exempt: their server is on-device. `.offline` classifies transient,
        // so no queued op is ever dropped for it.
        if connection.manualOffline, !isEmbeddedAccount {
            throw HalogenClient.ClientError.offline
        }
        return client
    }
}
