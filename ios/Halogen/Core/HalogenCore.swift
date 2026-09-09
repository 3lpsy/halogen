import Foundation
import Observation

/// App-wide core state: session lifecycle, local-profile FFI boot, API
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
    let libraryChanges = LibraryChanges()
    /// Set when a REMOTE session's auth died (401 the silent refresh couldn't
    /// heal): the connect page pre-fills this server + username so re-login
    /// is one password away (web: RootGuard redirects to Login with the
    /// server already known).
    private(set) var reauthHint: (serverUrl: String, username: String)?
    private var client: HalogenClient?
    /// In-flight auth-expiry recovery — collapses the burst of 401s a dead
    /// token produces into a single transition.
    private var reauthTask: Task<Void, Never>?
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
        do {
            guard let session = try SessionStore.loadDurably().active else {
                phase = .needsAuth
                return
            }
            await resume(session)
        } catch {
            DeviceLog.warn("session restore failed: \(error)")
            phase = .failed(error.localizedDescription)
        }
    }

    /// Resume any stored session (boot + account switch share this).
    /// Local-first: remote sessions go straight to `.ready` even offline.
    private func resume(_ session: Session) async {
        switch session.kind {
        case .embedded:
            do {
                try await startLocalProfile(as: session)
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
        guard case .embedded = account?.kind else {
            throw ConnectError.invalidUrl
        }
        let username = raw.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        guard username.count >= 3 else { throw EmbeddedUserError.tooShort }
        if SessionStore.load().sessions.contains(where: {
            $0.kind == .embedded && $0.username == username
        }) {
            throw EmbeddedUserError.exists(username)
        }

        _ = try await requireClient().createUser(
            username: username, password: UUID().uuidString, isAdmin: true)
        let session = Session(kind: .embedded, serverUrl: nil, username: username, token: "")
        teardownSession()
        phase = .starting
        try await startLocalProfile(as: session)
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
        try await destroyLocal(dataRoot: root.path)
        // The server-side users are gone — their silent-login credentials too.
        EmbeddedSecrets.clear()

        var registry = try SessionStore.loadDurably()
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
                guard let sub = session.localUserId.map(String.init) ?? AccountContext.jwtSub(session.token)
                else { continue }
                try? FileManager.default.removeItem(
                    at: clientRoot.appendingPathComponent("e\(sub)"))
            }
        }
        registry.sessions.removeAll { $0.kind == .embedded }
        if activeWasEmbedded {
            registry.activeId = registry.sessions.last?.id
        }
        try SessionStore.saveDurably(registry)

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
        try SessionStore.upsertActive(
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

    /// Landing page: use this device's own library (the local runtime).
    func useLocalLibrary() async {
        phase = .starting
        do {
            try await startLocalProfile(as: nil)
        } catch {
            DeviceLog.warn("embedded start failed — \(error)")
            phase = .failed(FriendlyError.message(error))
        }
    }

    /// Boot-failure screen: try the stored session again (a transient
    /// local-core failure shouldn't strand the user on an error page).
    func retryBoot() async {
        guard case .failed = phase else { return }
        phase = .idle
        await boot()
    }

    /// Explicitly rejected remote credentials return to sign-in while retaining the cache.
    /// Local profile rejection surfaces an unavailable-profile error.
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
            DeviceLog.error("local profile is unavailable")
            phase = .failed("Local profile is unavailable")
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
        libraryChanges.cancel()
        models?.player.stop()
        ArtLoader.shared.configure(token: nil)
        LocalTransport.remove(base: client?.base)
        pullTask?.cancel()
        pullTask = nil
        reauthTask?.cancel()
        reauthTask = nil
        reauthHint = nil
        client = nil
        account = nil
        store = nil
        if let outbox { Task { await outbox.setSuspended(true) } }
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

    func discoverPage(_ key: DiscoverSearchKey, cursor: String?) async throws -> DiscoverResultsPage {
        let client = try requireClient()
        switch key.mode {
        case .podcast:
            return .podcasts(try await client.discoverPodcastPage(query: key.query, providers: key.providers, cursor: cursor))
        case .episode:
            return .episodes(try await client.discoverEpisodePage(query: key.query, providers: key.providers, cursor: cursor))
        }
    }

    func discoverPodcast(feedURL: String, provider: DiscoverProvider) async throws -> DiscoverPodcastData {
        try await requireClient().discoverPodcast(feedURL: feedURL, provider: provider)
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
        guard !isEmbeddedAccount else { return nil }
        return try? requireClient().audioURL(episodeId: episodeId)
    }

    func localAudioURL(episodeId: Int32) async throws -> URL? {
        let client = try requireClient()
        let local = try LocalTransport.core(for: client.base)
        return try await local.audioPath(episodeId: episodeId).map { URL(fileURLWithPath: $0) }
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
        // local runtime is always reachable.
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
    /// (their media already lives on this device in the local library).
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
        let sourceStore = store
        let sourceModels = models
        let episodes = await sourceStore?.load([EpisodeData].self, key: CacheKey.podcastEpisodes(id)) ?? []
        guard store === sourceStore, await ensureQueued(.unsubscribe(podcastId: id)) else { return }
        for episode in episodes { sourceModels?.playbacks.removeProjected(episodeId: episode.id) }
        sourceModels?.podcasts.tombstoneProjected(id: id)
        sourceModels?.latest.removePodcastLocally(id)
        sourceModels?.downloads.removePodcastLocally(id)
        await sourceModels?.podcasts.refresh()
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

    /// Imported local profiles need no passwords; verify that each profile is available.
    func alignImportedUsers(_ usernames: [String]) async -> [String] {
        guard isEmbeddedAccount, let root = try? Self.embeddedRoot() else { return usernames }
        var failed: [String] = []
        for username in usernames {
            do {
                _ = try await startLocal(dataRoot: root.path, username: username)
            } catch {
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
        let source = try requireClient()
        let jobId = try await source.startPollJob()
        guard client?.tokenBox === source.tokenBox else { return jobId }
        libraryChanges.watch(jobId: jobId) {
            let job: PollJobData = try await source.get("admin/poll-job/\(jobId)")
            return job.status
        }
        return jobId
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

    /// The local runtime's on-disk library (Application Support container).
    private static func embeddedRoot() throws -> URL {
        let support = try FileManager.default.url(
            for: .applicationSupportDirectory,
            in: .userDomainMask,
            appropriateFor: nil,
            create: true
        )
        return support.appendingPathComponent("halogen-server", isDirectory: true)
    }

    /// Open the profile through FFI, retaining its existing cache identity.
    private func startLocalProfile(as session: Session?) async throws {
        let root = try Self.embeddedRoot()
        let local = try await startLocal(dataRoot: root.path, username: session?.username)
        let username = local.username()
        let url = LocalTransport.install(local)
        DeviceLog.shared.startCorePump()
        var saved = session ?? Session(kind: .embedded, serverUrl: nil, username: username, token: "")
        saved.username = username
        saved.token = ""
        saved.localUserId = local.userId()
        saved.password = nil
        try SessionStore.upsertActive(saved)
        await finish(
            client: HalogenClient(baseUrl: url), kind: .embedded,
            serverUrl: url, username: username, jwt: "",
            localAccount: AccountContext(kind: .embedded, userId: local.userId(), username: username),
            localIsAdmin: local.isAdmin())
    }

    private func finish(
        client: HalogenClient,
        kind: AccountContext.Kind,
        serverUrl: String,
        username: String,
        jwt: String,
        localAccount: AccountContext? = nil,
        localIsAdmin: Bool = false
    ) async {
        self.client = client
        self.baseUrl = serverUrl
        let account = localAccount ?? AccountContext.from(kind: kind, username: username, jwt: jwt)
        self.account = account
        // Admin gating (web: ClientConfig.is_admin): embedded users are
        // always admins by policy; remote resolves from the current user
        // after login. Best-effort and UI-only — the server is the authority.
        switch kind {
        case .embedded:
            isAdmin = localIsAdmin
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
            do {
                self.outbox = try await Outbox(
                    store: store,
                    perform: { queue in
                        if client.base.scheme == "halogen-local" {
                            return try await queue.drainLocal(core: LocalTransport.core(for: client.base))
                        }
                        let report = try await queue.drainRemote(baseUrl: serverUrl, token: client.token ?? "")
                        if report.authPaused, let account { _ = try await client.getUser(id: account.userId) }
                        return report
                    },
                    synchronize: { queue, cursor in
                        if client.base.scheme == "halogen-local" {
                            return try await queue.pullLocal(
                                core: LocalTransport.core(for: client.base), projectedCursor: cursor)
                        }
                        return try await queue.pullRemote(
                            baseUrl: serverUrl, token: client.token ?? "", projectedCursor: cursor)
                    },
                    onSnapshot: { [weak self, weak box = client.tokenBox] in
                        guard let self, let box, self.client?.tokenBox === box else { return }
                        await self.models?.podcasts.reloadSnapshot()
                        self.libraryChanges.invalidate()
                    },
                    onDeadLetter: { [weak self, weak box = client.tokenBox] op, error in
                        failures.record(op: op, error: error)
                        guard let self, let box, self.client?.tokenBox === box else { return }
                        await self.healAfterDeadLetter(op)
                    }
                )
            } catch {
                storageFailure = "Sync storage is unavailable. Pending changes have been preserved."
                DeviceLog.error("sync initialization failed: \(error)")
            }
        }
        ArtLoader.shared.configure(token: client.token, namespace: account?.namespace)
        let sessionId = SessionStore.load().activeId
        client.tokenBox.onRefresh = { [weak self, weak box = client.tokenBox] fresh in
            // An old request may refresh after an account switch; update its own session only.
            var registry = SessionStore.load()
            if let idx = registry.sessions.firstIndex(where: { $0.id == sessionId }) {
                registry.sessions[idx].token = fresh
                SessionStore.save(registry)
            }
            if let self, let box, self.client?.tokenBox === box {
                ArtLoader.shared.configure(token: fresh, namespace: self.account?.namespace)
            }
        }
        client.tokenBox.onAuthExpired = { [weak self, weak box = client.tokenBox] in
            Task { @MainActor in
                // Ignore stale sessions: an in-flight request from before an
                // account switch must not kick the NEW session to the
                // connect page.
                guard let self, let box, self.client?.tokenBox === box else { return }
                self.handleAuthExpired()
            }
        }
        reauthHint = nil
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
                await self.resyncAfterReconnect()
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
        await resyncAfterReconnect()
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
