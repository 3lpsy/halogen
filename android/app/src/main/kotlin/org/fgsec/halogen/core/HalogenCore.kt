package org.fgsec.halogen.core

import android.content.Context
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.async
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import org.fgsec.halogen.components.ArtLoader
import org.fgsec.halogen.components.ToastCenter
import org.fgsec.halogen.networking.*
import org.fgsec.halogen.networking.HalogenClient.PageOf
import org.fgsec.halogen.storage.AccountContext
import org.fgsec.halogen.storage.EmbeddedSecrets
import org.fgsec.halogen.storage.LocalStore
import org.fgsec.halogen.storage.Outbox
import org.fgsec.halogen.storage.OutboxOp
import org.fgsec.halogen.storage.CacheKey
import org.fgsec.halogen.storage.Session
import org.fgsec.halogen.storage.SessionStore
import org.fgsec.halogen.wire.DiscoverProvider
import org.fgsec.halogen.wire.DownloadProgressData
import org.fgsec.halogen.wire.EpisodeData
import org.fgsec.halogen.wire.OpmlImportResultData
import kotlinx.serialization.json.JsonObject
import org.fgsec.halogen.wire.OrderDirection
import org.fgsec.halogen.wire.PlaybackData
import org.fgsec.halogen.wire.PlaybackStatus
import org.fgsec.halogen.wire.PlaylistData
import org.fgsec.halogen.wire.PlaylistReorderField
import org.fgsec.halogen.wire.PodcastAutoPlaylistData
import org.fgsec.halogen.wire.PodcastConfigData
import org.fgsec.halogen.wire.PodcastConfigStoreData
import org.fgsec.halogen.wire.PodcastConfigUpdateData
import org.fgsec.halogen.wire.PodcastData
import org.fgsec.halogen.wire.PollJobData
import org.fgsec.halogen.wire.ServerErrorsData
import org.fgsec.halogen.wire.ServerLogsData
import org.fgsec.halogen.wire.UserData
import uniffi.halogen_mobile.EmbeddedCredentials
import uniffi.halogen_mobile.credentials
import uniffi.halogen_mobile.destroyEmbedded
import uniffi.halogen_mobile.initCore
import uniffi.halogen_mobile.recoverAdmin
import uniffi.halogen_mobile.recoverUser
import uniffi.halogen_mobile.startServer
import uniffi.halogen_mobile.waitReady
import java.io.File
import java.util.UUID

/// App-wide core state: session lifecycle, embedded-server FFI boot, the API
/// client, the account's LocalStore + Outbox, and the feature-model registry.
/// Views read `phase`; nothing below this file touches FFI or URLs directly.
class HalogenCore(val appContext: Context, val scope: CoroutineScope) {

    sealed class Phase {
        data object Idle : Phase()
        data object Starting : Phase()
        /** No stored session — show the landing page (server + credentials). */
        data object NeedsAuth : Phase()
        data object Ready : Phase()
        data class Failed(val message: String) : Phase()
    }

    var phase: Phase by mutableStateOf(Phase.Idle)
        private set
    var baseUrl: String? by mutableStateOf(null)
        private set
    /** Who's signed in — namespaces the LocalStore. An account switch swaps
     *  this + `store` + `models` (the web app remounts on its namespace
     *  change identically). */
    var account: AccountContext? by mutableStateOf(null)
        private set
    /** The active account's local cache. `null` only while signed out. */
    var store: LocalStore? by mutableStateOf(null)
        private set
    /** Offline mutation queue (drained whenever the server is reachable). */
    var outbox: Outbox? by mutableStateOf(null)
        private set
    var syncFailures: SyncFailures? by mutableStateOf(null)
        private set
    /** Set when the account's LocalStore failed to open — RootView shows a
     *  persistent banner (a dead store disables persistence AND the outbox). */
    var storageFailure: String? by mutableStateOf(null)
        private set
    /** Per-account feature models. Non-null exactly while phase == Ready. */
    var models: Models? by mutableStateOf(null)
        private set
    /** Whether the signed-in user is a server admin — UI gating only; the
     *  server is the real authority. Embedded users are always admins. */
    var isAdmin: Boolean by mutableStateOf(false)
        private set
    /** Navbar reachability dot. Probes whatever server the session points at. */
    val connection = ConnectionMonitor(scope)
    /** Set when a REMOTE session's auth died (401 the silent refresh couldn't
     *  heal): the connect page pre-fills this server + username. */
    var reauthHint: ReauthHint? by mutableStateOf(null)
        private set

    data class ReauthHint(val serverUrl: String, val username: String)

    private var client: HalogenClient? = null
    /** In-flight auth-expiry recovery — collapses the burst of 401s a dead
     *  token produces into a single transition. */
    private var reauthJob: Job? = null
    /** Embedded silent re-login is bounded (3 attempts) so a server that
     *  keeps rejecting fresh tokens degrades to signed-out, not a loop. */
    private var embeddedReloginAttempts = 0
    /** The periodic drain+pull loop — lives for the session, torn down with it. */
    private var pullJob: Job? = null

    val sessionStore = SessionStore(appContext)
    val embeddedSecrets = EmbeddedSecrets(appContext)

    /** Whether mutations should queue rather than go direct. */
    val isOffline: Boolean
        get() = connection.manualOffline || connection.status == ConnectionMonitor.Status.Offline

    // ── lifecycle ────────────────────────────────────────────────────────────

    /** App start: resume the stored session, or land on auth. Local-first: a
     *  stored REMOTE session resumes into Ready even if the server is
     *  unreachable — cached lists render and the dot shows offline. */
    suspend fun boot() {
        if (phase != Phase.Idle) return
        phase = Phase.Starting
        initCore()
        val session = sessionStore.load().active ?: run {
            phase = Phase.NeedsAuth
            return
        }
        resume(session)
    }

    /** Resume any stored session (boot + account switch share this). */
    private suspend fun resume(session: Session) {
        when (session.kind) {
            Session.Kind.EMBEDDED -> {
                try {
                    startEmbedded(session)
                } catch (e: Exception) {
                    DeviceLog.warn("embedded resume failed — $e")
                    phase = Phase.Failed(FriendlyError.message(e))
                }
            }
            Session.Kind.REMOTE -> {
                val serverUrl = session.serverUrl ?: run {
                    phase = Phase.NeedsAuth
                    return
                }
                val client = HalogenClient(baseUrl = serverUrl, token = session.token)
                finish(
                    client = client,
                    kind = AccountContext.Kind.Remote(serverUrl),
                    serverUrl = serverUrl,
                    username = session.username,
                    jwt = session.token,
                )
            }
        }
    }

    /** Settings → Accounts: activate a different saved session. */
    suspend fun switchAccount(session: Session) {
        sessionStore.switchTo(session.id)
        teardownSession()
        phase = Phase.Starting
        resume(session)
    }

    /** Settings → Accounts: land on the connect page WITHOUT dropping saved
     *  sessions (the new login upserts into the registry). */
    fun beginAddAccount() {
        addAccountReturnSession = sessionStore.load().active
        teardownSession()
        phase = Phase.NeedsAuth
    }

    /** The session to return to when the add-account landing is cancelled.
     *  Set ONLY by beginAddAccount — an expired session offers no way back. */
    var addAccountReturnSession: Session? = null
        private set

    /** Back out of add-account: resume the session the user came from. */
    suspend fun cancelAddAccount() {
        val back = addAccountReturnSession ?: return
        addAccountReturnSession = null
        switchAccount(back)
    }

    class EmbeddedUserException(message: String) : Exception(message)

    /** Create + switch to a new user on the EMBEDDED server (username only —
     *  the app generates and stores the password). Lowercase + 3-char minimum
     *  + duplicate guard, admin by policy, secret persisted only after the
     *  server-side create succeeds. */
    suspend fun addEmbeddedUser(usernameRaw: String) {
        val base = baseUrl
        if (base == null || account?.kind !is AccountContext.Kind.Embedded) {
            throw ConnectException()
        }
        val username = usernameRaw.trim().lowercase()
        if (username.length < 3) throw EmbeddedUserException("Username must be at least 3 characters")
        if (sessionStore.load().sessions.any { it.kind == Session.Kind.EMBEDDED && it.username == username }) {
            throw EmbeddedUserException("'$username' already exists — switch to it instead")
        }

        val stored = embeddedSecrets.password(username)
        val password: String
        if (stored != null) {
            // Known credential without a registered session (the account was
            // removed): reconnect with it — a fresh create would 409.
            password = stored
        } else {
            password = UUID.randomUUID().toString()
            requireClient().createUser(username = username, password = password, isAdmin = true)
            embeddedSecrets.remember(username, password)
        }
        val client = HalogenClient(baseUrl = base)
        val jwt = client.login(username = username, password = password)
        val session = Session(
            kind = Session.Kind.EMBEDDED, serverUrl = null, username = username,
            token = jwt, password = password,
        )
        sessionStore.upsertActive(session)
        teardownSession()
        phase = Phase.Starting
        resume(session)
    }

    /** Recreate the per-account models over the same session (post-purge). */
    fun remountModels() {
        val m = Models(this)
        models = m
        scope.launch { m.nav.load() }
        scope.launch { m.swipes.load() }
        scope.launch { m.prefs.load() }
        scope.launch { m.device.load() }
        scope.launch { m.playbacks.load() }
        scope.launch { m.podcasts.seed() }
        scope.launch { m.playlists.seed() }
        // Queue resolves at mount (cache-first) so "Add to Queue" works from
        // any tab before the Queue screen has ever been opened.
        scope.launch { m.queue.load() }
    }

    /** Local Data: destroy the embedded library (stops the in-process server
     *  and deletes db/media/secrets). Embedded sessions leave the registry;
     *  an active embedded account signs out to the next account or landing. */
    suspend fun destroyEmbeddedServer() {
        val root = embeddedRoot()
        destroyEmbedded(dataRoot = root.path)
        root.deleteRecursively()
        // The server-side users are gone — their silent-login credentials too.
        embeddedSecrets.clear()

        var registry = sessionStore.load()
        val activeWasEmbedded = registry.active?.kind == Session.Kind.EMBEDDED
        // The embedded sessions' CLIENT caches must die with the library — a
        // re-created embedded user with the same id would inherit stale
        // lists/prefs from the old world otherwise.
        val clientRoot = File(appContext.filesDir, "halogen-client")
        for (session in registry.sessions.filter { it.kind == Session.Kind.EMBEDDED }) {
            val sub = AccountContext.jwtSub(session.token) ?: continue
            File(clientRoot, "e$sub").deleteRecursively()
        }
        registry = registry.copy(sessions = registry.sessions.filterNot { it.kind == Session.Kind.EMBEDDED })
        if (activeWasEmbedded) {
            registry = registry.copy(activeId = registry.sessions.lastOrNull()?.id)
        }
        sessionStore.save(registry)

        if (activeWasEmbedded) {
            teardownSession()
            val next = registry.active
            if (next != null) {
                phase = Phase.Starting
                resume(next)
            } else {
                phase = Phase.NeedsAuth
            }
        }
    }

    fun renameActiveAccount(username: String) {
        sessionStore.renameActive(username)
        account?.let {
            account = AccountContext(kind = it.kind, userId = it.userId, username = username)
        }
    }

    class ConnectException : Exception("Server URL must start with http:// or https://")

    /** Landing page: connect to a remote server — reachability probe, then
     *  login. Throws so the form can show the failure inline. */
    suspend fun connectRemote(serverUrl: String, username: String, password: String) {
        val normalized = normalizeServerUrl(serverUrl)
        val scheme = normalized.substringBefore("://", "")
        if (scheme != "http" && scheme != "https") throw ConnectException()
        val client = HalogenClient(baseUrl = normalized)
        client.health()
        val jwt = client.login(username = username, password = password)
        sessionStore.upsertActive(
            Session(kind = Session.Kind.REMOTE, serverUrl = normalized, username = username, token = jwt)
        )
        finish(
            client = client,
            kind = AccountContext.Kind.Remote(normalized),
            serverUrl = normalized,
            username = username,
            jwt = jwt,
        )
    }

    /** Landing page: use this device's own library (the embedded server). */
    suspend fun useLocalLibrary() {
        phase = Phase.Starting
        try {
            startEmbedded(null)
        } catch (e: Exception) {
            DeviceLog.warn("embedded start failed — $e")
            phase = Phase.Failed(FriendlyError.message(e))
        }
    }

    /** Boot-failure screen: try the stored session again. */
    suspend fun retryBoot() {
        if (phase !is Phase.Failed) return
        phase = Phase.Idle
        boot()
    }

    /** A 401 the token refresh couldn't heal — auth is dead. Embedded
     *  accounts re-login silently with their on-disk credentials; remote
     *  accounts return to the connect page with the session and cache KEPT. */
    private fun handleAuthExpired() {
        if (phase != Phase.Ready || reauthJob != null) return
        reauthJob = scope.launch {
            recoverExpiredAuth()
            reauthJob = null
        }
    }

    private suspend fun recoverExpiredAuth() {
        val account = account
        if (phase != Phase.Ready || account == null) return
        when (val kind = account.kind) {
            is AccountContext.Kind.Embedded -> {
                embeddedReloginAttempts += 1
                if (embeddedReloginAttempts <= 3) {
                    var password = embeddedSecrets.password(account.username)
                    if (password == null) {
                        // No stored credential (pre-gating server-side rename):
                        // rotate the row in the DB and adopt the fresh secret.
                        val creds = runCatching {
                            recoverUser(dataRoot = embeddedRoot().path, username = account.username)
                        }.getOrNull()
                        if (creds != null) {
                            embeddedSecrets.remember(creds.username, creds.password)
                            password = creds.password
                        }
                    }
                    if (password != null) {
                        val jwt = runCatching {
                            client?.login(username = account.username, password = password)
                        }.getOrNull()
                        if (jwt != null) {
                            // login() already refreshed the shared TokenBox; mirror
                            // it into the saved session + art loader like a refresh.
                            client?.tokenBox?.onRefresh?.invoke(jwt)
                            DeviceLog.info("auth: embedded session re-authenticated silently")
                            return
                        }
                    }
                }
                // Out of budget (or the secret is gone): degrade to the landing
                // page. The session stays saved — next boot retries the resume.
                DeviceLog.warn("auth: embedded silent re-login failed")
                teardownSession()
                phase = Phase.NeedsAuth
            }
            is AccountContext.Kind.Remote -> {
                DeviceLog.warn("auth: remote session expired; returning to connect page")
                val username = account.username
                teardownSession()
                reauthHint = ReauthHint(serverUrl = kind.serverUrl, username = username)
                phase = Phase.NeedsAuth
                ToastCenter.error("Session expired — sign in again")
            }
        }
    }

    /** Remove the active account. Another saved account takes over if one
     *  exists; otherwise back to the landing page. Cached data stays on disk. */
    fun signOut() {
        val registry = sessionStore.load()
        val next = registry.activeId?.let { sessionStore.remove(it) }
        teardownSession()
        if (next != null) {
            phase = Phase.Starting
            scope.launch { resume(next) }
        } else {
            phase = Phase.NeedsAuth
        }
    }

    private fun teardownSession() {
        // Unlike iOS (whose player dies with its model), the ExoPlayer is a
        // shared process singleton — without an explicit detach, audio keeps
        // playing behind the landing page after sign-out.
        models?.player?.detach()
        pullJob?.cancel()
        pullJob = null
        reauthJob?.cancel()
        reauthJob = null
        reauthHint = null
        client = null
        account = null
        store = null
        outbox = null
        syncFailures = null
        storageFailure = null
        models = null
        baseUrl = null
        isAdmin = false
        connection.stop()
    }

    // ── typed fetches (thin passthroughs so models never hold a client) ──────

    suspend fun podcasts(page: Int = 0): PageOf<PodcastData> = requireClient().podcasts(page)

    suspend fun episodes(
        podcastId: Int, extra: List<Pair<String, String>> = emptyList(), page: Int = 0,
    ): PageOf<EpisodeData> = requireClient().episodes(podcastId, extra, page)

    /** Raw facet variant (Downloads page and friends supply their own params). */
    suspend fun latestEpisodesRaw(
        extra: List<Pair<String, String>>, page: Int = 0, pageSize: Int = 20,
    ): PageOf<EpisodeData> = requireClient().latestEpisodes(extra, page, pageSize)

    suspend fun playlistsList(): List<PlaylistData> = requireClient().playlists()

    suspend fun defaultPlaylist(): PlaylistData? = requireClient().defaultPlaylist()

    suspend fun playlistEpisodes(playlistId: Int): List<EpisodeData> =
        requireClient().playlistEpisodes(playlistId)

    suspend fun createPlaylist(
        name: String, description: String? = null, isDefault: Boolean,
        deleteServerFile: Boolean = false, deleteClientFile: Boolean = false,
    ): PlaylistData = requireClient().createPlaylist(name, description, isDefault, deleteServerFile, deleteClientFile)

    suspend fun deletePlaylist(id: Int) = requireClient().deletePlaylist(id)

    /** Full playlist edit (nil = leave unchanged). */
    suspend fun updatePlaylist(
        id: Int, name: String?, description: String?, isDefault: Boolean?,
        deleteServerFile: Boolean?, deleteClientFile: Boolean?,
    ) {
        requireClient().updatePlaylist(id, name, isDefault, description, deleteServerFile, deleteClientFile)
        if (isDefault == true) models?.queue?.refresh()
    }

    suspend fun makeQueuePlaylist(id: Int) {
        requireClient().makeQueuePlaylist(id)
        models?.queue?.refresh()
    }

    suspend fun reorderPlaylist(id: Int, field: PlaylistReorderField, direction: OrderDirection) =
        requireClient().reorderPlaylist(id, field, direction)

    suspend fun episodeDetail(id: Int): EpisodeData = requireClient().episode(id)

    /** Overlay-wins played-status read for local list filtering. */
    fun overlayStatus(episode: EpisodeData): PlaybackStatus =
        models?.playbacks?.status(episode) ?: episode.playback_status ?: PlaybackStatus.Unplayed

    /** Deep-link fetch-throughs: resolve a by-id target the pools don't hold. */
    suspend fun podcast(id: Int): PodcastData = requireClient().podcast(id)

    suspend fun playlist(id: Int): PlaylistData = requireClient().playlist(id)

    /** One page of playback rows, updated_at desc (the History source). */
    suspend fun playbacksPage(page: Int, pageSize: Int = 20): PageOf<PlaybackData> =
        requireClient().playbacks(page, pageSize)

    suspend fun discoverSearch(query: String, providers: List<DiscoverProvider>? = null) =
        requireClient().discoverSearch(query, providers)

    suspend fun discoverProviders() = requireClient().discoverProviders()

    suspend fun subscribePodcast(title: String, feedUrl: String, description: String?): PodcastData =
        requireClient().createPodcast(title = title, feedUrl = feedUrl, description = description)

    /** The streaming audio URL for the player (`null` while signed out). */
    fun audioUrl(episodeId: Int): okhttp3.HttpUrl? =
        runCatching { requireClient() }.getOrNull()?.audioUrl(episodeId)

    /** The raw API JWT — the player's data source needs it as a literal header. */
    val apiToken: String?
        get() = client?.token

    /** The navbar's manual offline toggle: pause probing + outbox drains;
     *  flipping back online probes and drains immediately. */
    fun setManualOffline(offline: Boolean) {
        connection.applyManualOffline(offline)
        scope.launch { outbox?.setSuspended(offline) }
        if (!offline) scope.launch { outbox?.drain() }
        // Persisted: a relaunch must come back in the chosen mode. Embedded
        // never persists true — the on-device server is always reachable.
        val persisted = offline && !isEmbeddedAccount
        val store = store
        scope.launch { store?.save(persisted, CacheKey.manualOffline) }
        DeviceLog.info(if (offline) "went manually offline" else "back online (manual)")
    }

    /** Boot-time re-apply of the persisted manual-offline choice. */
    suspend fun restoreManualOffline() {
        if (isEmbeddedAccount) return
        if (store?.load<Boolean>(CacheKey.manualOffline) != true) return
        connection.applyManualOffline(true)
        outbox?.setSuspended(true)
        DeviceLog.info("restored manual offline from last session")
    }

    val isEmbeddedAccount: Boolean
        get() = account?.kind is AccountContext.Kind.Embedded

    /** The strategy playback actually uses: embedded accounts always stream. */
    val effectivePlaybackStrategy: ClientPrefs.PlaybackStrategy
        get() = if (isEmbeddedAccount) ClientPrefs.PlaybackStrategy.StreamOnly
        else models?.prefs?.prefs?.playbackStrategy ?: ClientPrefs.PlaybackStrategy.DownloadOnly

    suspend fun rawJson(path: String): JsonObject = requireClient().rawJson(path)

    /** Admin: create a server user with an explicit password. */
    suspend fun createUser(username: String, password: String, isAdmin: Boolean) {
        requireClient().createUser(username = username, password = password, isAdmin = isAdmin)
    }

    suspend fun configOverrides(): ConfigOverridesData = requireClient().configOverrides()

    suspend fun setConfigOverrides(overrides: ConfigOverridesData) =
        requireClient().setConfigOverrides(overrides)

    suspend fun clearConfigOverrides() = requireClient().clearConfigOverrides()

    suspend fun updatePodcast(id: Int, title: String?, description: String?, feedUrl: String?) =
        requireClient().updatePodcast(id, title, description, feedUrl)

    suspend fun deletePodcast(id: Int) = requireClient().deletePodcast(id)

    /** Unsubscribe, offline-capable: tombstone + optimistic removal, local
     *  cascade, queued delete, then a reconcile refresh. */
    suspend fun unsubscribePodcast(id: Int) {
        models?.podcasts?.tombstone(id)
        purgePodcastLocalData(id)
        outbox?.enqueue(OutboxOp.Kind.Unsubscribe(podcastId = id))
        models?.podcasts?.refresh()
    }

    /** Prune every local trace of an unsubscribed podcast so no snapshot can
     *  rehydrate it. Episode ids resolve from the podcast's episode snapshot
     *  FIRST (playbacks are keyed by episode id). */
    private suspend fun purgePodcastLocalData(podcastId: Int) {
        val store = store ?: return
        val episodes = store.load<List<EpisodeData>>(CacheKey.podcastEpisodes(podcastId)) ?: emptyList()
        for (episode in episodes) {
            models?.playbacks?.purge(episode.id)
            store.remove(CacheKey.episode(episode.id))
            store.remove(CacheKey.metadata("episodes/${episode.id}"))
        }
        store.remove(CacheKey.podcastEpisodes(podcastId))
        store.remove(CacheKey.autoPlaylists(podcastId))
        store.remove(CacheKey.metadata("podcasts/$podcastId"))
        // Cross-podcast list snapshots: their prefix-merge would otherwise
        // keep this podcast's rows as tail forever.
        for (key in store.listKeys()) {
            if (!(key.startsWith("latest-") || key.startsWith("downloads-"))) continue
            if (key == CacheKey.latestScrollAnchor) continue
            val rows = store.load<List<EpisodeData>>(key) ?: continue
            val filtered = rows.filterNot { it.podcast_id == podcastId }
            if (filtered.size != rows.size) store.save(filtered, key)
        }
        models?.latest?.removePodcastLocally(podcastId)
        models?.downloads?.removePodcastLocally(podcastId)
    }

    suspend fun createPodcastConfig(podcastId: Int, data: PodcastConfigStoreData) =
        requireClient().createPodcastConfig(podcastId, data)

    suspend fun podcastConfig(id: Int): PodcastConfigData = requireClient().podcastConfig(id)

    suspend fun updatePodcastConfig(configId: Int, data: PodcastConfigUpdateData) =
        requireClient().updatePodcastConfig(configId, data)

    suspend fun deletePodcastConfig(podcastId: Int) = requireClient().deletePodcastConfig(podcastId)

    suspend fun autoPlaylists(podcastId: Int): List<PodcastAutoPlaylistData> =
        requireClient().autoPlaylists(podcastId)

    suspend fun setAutoPlaylists(podcastId: Int, playlistIds: List<Int>, addToStart: Boolean?) =
        requireClient().setAutoPlaylists(podcastId, playlistIds, addToStart)

    suspend fun updateUsername(userId: Int, username: String) =
        requireClient().updateUsername(userId, username)

    // ── admin user management ────────────────────────────────────────────────

    suspend fun listUsers(): List<UserData> = requireClient().listUsers()

    suspend fun updateUser(userId: Int, username: String?, isAdmin: Boolean?) =
        requireClient().updateUser(userId, username, isAdmin)

    suspend fun deleteUser(id: Int) = requireClient().deleteUser(id)

    suspend fun dbExport(): Pair<ByteArray, String> = requireClient().dbExport()

    suspend fun dbImport(payload: ByteArray): DbImportSummaryData = requireClient().dbImport(payload)

    /** After an embedded DB import: rotate each imported user's random
     *  password into EmbeddedSecrets so switching to them silently works.
     *  Best-effort per user; returns the usernames that couldn't be aligned. */
    suspend fun alignImportedUsers(usernames: List<String>): List<String> {
        if (!isEmbeddedAccount) return usernames
        val root = embeddedRoot()
        val failed = mutableListOf<String>()
        for (username in usernames) {
            try {
                val creds = recoverUser(dataRoot = root.path, username = username)
                embeddedSecrets.remember(creds.username, creds.password)
            } catch (e: Exception) {
                DeviceLog.warn("db import: couldn't align '$username' for silent login: $e")
                failed.add(username)
            }
        }
        return failed
    }

    suspend fun opmlExport(): String = requireClient().opmlExport()

    suspend fun opmlImport(opml: String): OpmlImportResultData = requireClient().opmlImport(opml)

    suspend fun pollJobs(): List<PollJobData> = requireClient().pollJobs()

    suspend fun startPollJob(): ULong = requireClient().startPollJob()

    suspend fun serverLogs(): ServerLogsData = requireClient().serverLogs()

    suspend fun serverErrors(): ServerErrorsData = requireClient().serverErrors()

    suspend fun changePassword(current: String, new: String) =
        requireClient().changePassword(current, new)

    suspend fun triggerDownload(episodeId: Int) = requireClient().triggerDownload(episodeId)

    suspend fun removeServerDownload(episodeId: Int) = requireClient().removeServerDownload(episodeId)

    suspend fun downloadProgress(episodeId: Int): DownloadProgressData? =
        requireClient().downloadProgress(episodeId)

    // ── media URLs (the server art cache; ArtLoader adds the bearer) ─────────

    fun podcastArtUrl(podcast: PodcastData, small: Boolean = true): String? =
        artUrl("podcasts", podcast.id, small)

    fun episodeArtUrl(episode: EpisodeData, small: Boolean = true): String? =
        artUrl("episodes", episode.id, small)

    // ── internals ────────────────────────────────────────────────────────────

    /** Which self-heal a failed embedded login triggers: the seeded-admin
     *  path recovers via recoverAdmin (which also ADOPTS a renamed admin
     *  row); a specific user rotates their own row, falling back to admin
     *  adoption when the row under that name is gone. */
    private sealed class EmbeddedRecovery {
        data object Admin : EmbeddedRecovery()
        data class User(val username: String) : EmbeddedRecovery()
    }

    private suspend fun recoverEmbeddedCredentials(
        root: String, recovery: EmbeddedRecovery,
    ): EmbeddedCredentials = when (recovery) {
        is EmbeddedRecovery.Admin -> recoverAdmin(dataRoot = root)
        is EmbeddedRecovery.User -> try {
            recoverUser(dataRoot = root, username = recovery.username)
        } catch (e: Exception) {
            DeviceLog.warn("embedded: recoverUser failed — ${e::class.simpleName}: ${e.message}")
            recoverAdmin(dataRoot = root)
        }
    }

    /** The embedded server's on-disk library (app-private files dir). */
    fun embeddedRoot(): File = File(appContext.filesDir, "halogen-server")

    /** Boot (or reuse) the in-process server, then silent-login: the
     *  provisioned admin by default, or a stored embedded user's app-managed
     *  credentials. Missing or rejected credentials self-heal by rotating the
     *  row's password — an embedded account never lands on a password prompt. */
    private suspend fun startEmbedded(session: Session?) {
        val root = embeddedRoot()
        root.mkdirs()

        val url = startServer(dataRoot = root.path)
        waitReady()
        // The embedded server's tracing is only visible through the core's
        // device-log ring — start folding it into the native Device Logs.
        DeviceLog.shared.startCorePump()

        val recovery: EmbeddedRecovery =
            session?.let { EmbeddedRecovery.User(it.username) } ?: EmbeddedRecovery.Admin
        var username: String
        var password: String
        val storedSecret = session?.let { embeddedSecrets.password(it.username) }
        when {
            session?.password != null -> {
                username = session.username
                password = session.password
            }
            session != null && storedSecret != null -> {
                // The app-side secrets mirror (survives session removal).
                username = session.username
                password = storedSecret
            }
            session != null -> {
                // A stored session with NO credential anywhere: rotate the row
                // in the DB instead of stranding the user on the landing page.
                val creds = recoverEmbeddedCredentials(root.path, recovery)
                username = creds.username
                password = creds.password
            }
            else -> {
                val creds = credentials(dataRoot = root.path)
                username = creds.username
                password = creds.password
            }
        }
        val client = HalogenClient(baseUrl = url)
        val jwt: String = try {
            client.login(username = username, password = password)
        } catch (e: Exception) {
            // Credential drift (stale secrets, out-of-band change): rotate the
            // password in the DB and retry once.
            DeviceLog.warn("auth: embedded credentials rejected — running recovery")
            val fresh = recoverEmbeddedCredentials(root.path, recovery)
            username = fresh.username
            password = fresh.password
            client.login(username = username, password = password)
        }
        // Mirror the working credential into the app-side secrets store so a
        // later session removal never strands this account.
        embeddedSecrets.remember(username, password)
        val saved = (session ?: Session(
            kind = Session.Kind.EMBEDDED, serverUrl = null, username = username, token = jwt,
        )).copy(username = username, password = password, token = jwt)
        sessionStore.upsertActive(saved)
        finish(
            client = client,
            kind = AccountContext.Kind.Embedded,
            serverUrl = url,
            username = username,
            jwt = jwt,
        )
    }

    private suspend fun finish(
        client: HalogenClient,
        kind: AccountContext.Kind,
        serverUrl: String,
        username: String,
        jwt: String,
    ) {
        this.client = client
        this.baseUrl = serverUrl
        val account = AccountContext.from(kind = kind, username = username, jwt = jwt)
        this.account = account
        // Admin gating: embedded users are always admins by policy; remote
        // resolves from the current user after login. Best-effort, UI-only.
        when (kind) {
            is AccountContext.Kind.Embedded -> isAdmin = true
            is AccountContext.Kind.Remote -> {
                isAdmin = false
                if (account != null) {
                    scope.launch {
                        val me = runCatching { client.getUser(account.userId) }.getOrNull()
                        isAdmin = me?.is_admin ?: false
                    }
                }
            }
        }
        // "anon" only if the JWT is somehow malformed — a working
        // un-namespaced cache beats a crash.
        val store: LocalStore? = try {
            LocalStore(appContext, account?.namespace ?: "anon").also { storageFailure = null }
        } catch (e: Exception) {
            // A dead store silently no-ops EVERY optimistic mutation (null
            // outbox) — say so loudly and persistently (RootView banner).
            storageFailure = "Local storage is unavailable — changes made here won't be saved or synced."
            DeviceLog.error("localstore: init failed — $e")
            null
        }
        this.store = store
        if (store != null) {
            val failures = SyncFailures(store)
            this.syncFailures = failures
            this.outbox = Outbox.create(
                store = store,
                perform = { op -> execute(op) },
                onDeadLetter = { op, error ->
                    failures.record(op, error)
                    healAfterDeadLetter(op.kind)
                },
            )
        }
        ArtLoader.configure(context = appContext, token = client.token, namespace = account?.namespace)
        client.tokenBox.onRefresh = { fresh ->
            // Keep the persisted session + art loader on the fresh token.
            val registry = sessionStore.load()
            registry.sessions.firstOrNull { it.id == registry.activeId }?.let { active ->
                sessionStore.save(
                    registry.copy(sessions = registry.sessions.map {
                        if (it.id == active.id) it.copy(token = fresh) else it
                    })
                )
            }
            // Keep the namespace: dropping it rebuilt the loader WITHOUT its
            // per-account disk cache after the first token refresh.
            ArtLoader.configure(
                context = appContext, token = fresh, namespace = this.account?.namespace,
            )
        }
        val box = client.tokenBox
        client.tokenBox.onAuthExpired = {
            scope.launch {
                // Ignore stale sessions: an in-flight request from before an
                // account switch must not kick the NEW session out.
                if (this@HalogenCore.client?.tokenBox === box) handleAuthExpired()
            }
        }
        reauthHint = null
        embeddedReloginAttempts = 0
        connection.onOnline = { scope.launch { resyncAfterReconnect() } }
        connection.start(baseUrl = serverUrl)
        val m = Models(this)
        this.models = m
        scope.launch { m.nav.load() }
        scope.launch { m.swipes.load() }
        scope.launch { m.prefs.load() }
        scope.launch {
            m.device.load()
            // Staged partials continue without a manual tap.
            m.device.resumePartials()
        }
        scope.launch { m.playbacks.load() }
        // Pool hydration: podcasts + playlists seed cache-only so by-id
        // navigation and row-menu playlist toggles work from any tab, offline
        // included — no network until the owning screen revalidates.
        scope.launch { m.podcasts.seed() }
        scope.launch { m.playlists.seed() }
        // Queue resolves at boot (cache-first) so "Add to Queue" works from
        // any tab — including offline cold starts.
        scope.launch { m.queue.load() }
        // Restore BEFORE the first drain, or the drain races the suspension.
        scope.launch {
            restoreManualOffline()
            outbox?.drain()
        }
        startPeriodicPull()
        // Any session reaching ready invalidates a pending add-account return.
        addAccountReturnSession = null
        phase = Phase.Ready
    }

    /** A dropped op leaves optimistic state lying: reconcile promptly, while
     *  the dead-letter toast explains why. */
    private suspend fun healAfterDeadLetter(op: OutboxOp.Kind) {
        val models = models ?: return
        when (op) {
            is OutboxOp.Kind.Subscribe -> {
                models.discover.noteSubscribeFailed(op.feedUrl)
                if (models.podcasts.loaded) models.podcasts.refresh()
            }
            is OutboxOp.Kind.Unsubscribe -> {
                if (models.podcasts.loaded) models.podcasts.refresh()
            }
            is OutboxOp.Kind.AddToPlaylist, is OutboxOp.Kind.RemoveFromPlaylist,
            is OutboxOp.Kind.MoveInPlaylist, is OutboxOp.Kind.ReorderPlaylist -> {
                models.queue.refresh()
                if (models.playlists.loaded) models.playlists.refresh()
            }
            else -> {}
        }
    }

    /** The 60s tick: drain queued ops, then revalidate the queue. */
    private fun startPeriodicPull() {
        pullJob?.cancel()
        pullJob = scope.launch {
            while (isActive) {
                delay(60_000)
                if (!isActive) return@launch
                if (connection.status != ConnectionMonitor.Status.Online) continue
                outbox?.drain()
                models?.queue?.refresh()
            }
        }
    }

    /** Foreground return: probe reachability AND push/pull immediately — the
     *  60s tick and the offline→online transition both miss "came back to a
     *  backgrounded app whose network never changed". */
    suspend fun foregroundSync() {
        connection.probe()
        if (phase != Phase.Ready) return
        outbox?.drain()
        models?.queue?.refresh()
    }

    /** Back online: drain BEFORE pulling, then revalidate mounted screens so
     *  stale/errored content heals without a manual pull-to-refresh. */
    private suspend fun resyncAfterReconnect() {
        outbox?.drain()
        // Admin gating resolved over a dead connection sticks false for the
        // whole session — re-resolve when healthy.
        val account = account
        val client = client
        if (account?.kind is AccountContext.Kind.Remote && !isAdmin && client != null) {
            runCatching { client.getUser(account.userId) }.getOrNull()?.let {
                isAdmin = it.is_admin
            }
        }
        val models = models ?: return
        if (models.queue.loaded) models.queue.refresh()
        if (models.playlists.loaded) models.playlists.refresh()
        coroutineScope {
            val jobs = listOf(
                async { if (models.latest.loaded) models.latest.refresh() },
                async { if (models.podcasts.loaded) models.podcasts.refresh() },
                async { if (models.history.loaded) models.history.refresh() },
                async { if (models.downloads.loaded) models.downloads.refresh() },
            )
            jobs.forEach { it.await() }
        }
    }

    /** The Outbox's executor — one queued op against the API. */
    private suspend fun execute(op: OutboxOp) {
        val client = requireClient()
        when (val k = op.kind) {
            is OutboxOp.Kind.AddToPlaylist ->
                client.addEpisode(k.playlistId, k.episodeId, k.position)
            is OutboxOp.Kind.RemoveFromPlaylist ->
                client.removeEpisode(k.playlistId, k.episodeId)
            is OutboxOp.Kind.MoveInPlaylist ->
                client.moveEpisode(k.playlistId, k.episodeId, k.to)
            is OutboxOp.Kind.SetCursor ->
                client.upsertPlayback(k.episodeId, cursor = k.cursor, completed = false)
            // Same shape the web outbox sends: cursor 0 + the completed flag.
            is OutboxOp.Kind.SetPlayed ->
                client.upsertPlayback(k.episodeId, cursor = 0u, completed = k.played)
            is OutboxOp.Kind.ReorderPlaylist ->
                client.reorderPlaylist(k.playlistId, k.field, k.direction)
            is OutboxOp.Kind.UpdatePlaylist ->
                client.updatePlaylist(
                    k.playlistId, k.name, k.isDefault, k.description,
                    k.deleteServerFile, k.deleteClientFile,
                )
            is OutboxOp.Kind.MovePlaylist -> client.movePlaylist(k.playlistId, k.to)
            is OutboxOp.Kind.Subscribe -> {
                // The DTO requires a 1-256 char title; fall back to the feed
                // URL — the RSS ingest heals it to the channel title. Clamp
                // the description too: an oversize blurb would 422 and
                // permanently drop the queued subscription.
                val trimmed = k.title?.trim() ?: ""
                client.createPodcast(
                    title = (trimmed.ifEmpty { k.feedUrl }).take(256),
                    feedUrl = k.feedUrl,
                    description = k.description?.take(4096),
                )
            }
            is OutboxOp.Kind.Unsubscribe -> client.deletePodcast(k.podcastId)
            is OutboxOp.Kind.TriggerDownload -> client.triggerDownload(k.episodeId)
            is OutboxOp.Kind.RemoveServerDownload -> client.removeServerDownload(k.episodeId)
            is OutboxOp.Kind.UpdatePodcastConfig -> client.updatePodcastConfig(k.configId, k.data)
            is OutboxOp.Kind.RemovePodcastConfig -> client.deletePodcastConfig(k.podcastId)
            is OutboxOp.Kind.SetAutoPlaylists ->
                client.setAutoPlaylists(k.podcastId, k.playlistIds, k.addToStart)
        }
    }

    private fun artUrl(kind: String, id: Int, small: Boolean): String? {
        val base = baseUrl ?: return null
        val suffix = if (small) "/art/small" else "/art"
        return "$base/api/v1/$kind/$id$suffix"
    }

    private fun normalizeServerUrl(raw: String): String = raw.trim().trimEnd('/')

    fun requireClient(): HalogenClient {
        val client = client ?: throw HalogenClient.ClientError.SignedOut
        // Manual offline is a HARD gate on every API read/write. Embedded
        // accounts are exempt — their server is on-device.
        if (connection.manualOffline && !isEmbeddedAccount) {
            throw HalogenClient.ClientError.Offline
        }
        return client
    }
}
