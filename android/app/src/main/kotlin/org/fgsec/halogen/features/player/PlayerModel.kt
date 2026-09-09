package org.fgsec.halogen.features.player

import android.graphics.Bitmap
import android.net.Uri
import androidx.annotation.OptIn
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.media3.common.C
import androidx.media3.common.MediaItem
import androidx.media3.common.MediaMetadata
import androidx.media3.common.PlaybackException
import androidx.media3.common.Player
import androidx.media3.common.util.UnstableApi
import androidx.media3.datasource.HttpDataSource
import androidx.media3.exoplayer.ExoPlayer
import coil3.request.ImageRequest
import coil3.request.SuccessResult
import coil3.toBitmap
import java.io.ByteArrayOutputStream
import java.net.SocketTimeoutException
import java.net.UnknownHostException
import kotlin.math.ceil
import kotlin.math.max
import kotlin.math.min
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.fgsec.halogen.components.ArtLoader
import org.fgsec.halogen.core.ClientPrefs
import org.fgsec.halogen.core.DeviceDownloads
import org.fgsec.halogen.core.DeviceLog
import org.fgsec.halogen.core.HalogenCore
import org.fgsec.halogen.player.PlaybackService
import org.fgsec.halogen.player.PlayerBridge
import org.fgsec.halogen.storage.CacheKey
import org.fgsec.halogen.wire.DownloadStatus
import org.fgsec.halogen.wire.EpisodeChapterData
import org.fgsec.halogen.wire.EpisodeData

/// Playback: Media3 ExoPlayer streaming from the server (bearer via shared OkHttp),
/// background audio through PlaybackService, cursors through the outbox, continuation
/// via the play-context playlist or the queue. Mirrors ios Features/Player/PlayerModel.swift.
@OptIn(UnstableApi::class)
class PlayerModel(private val core: HalogenCore) {
    private val accountStore = core.store

    private val scope: CoroutineScope get() = core.scope

    var current: EpisodeData? by mutableStateOf(null)
        private set
    var isPlaying: Boolean by mutableStateOf(false)
        private set
    var duration: Double by mutableStateOf(0.0)
        private set
    /** Server-download progress while an un-downloaded episode is prepared. */
    var preparing: Boolean by mutableStateOf(false)
        private set
    /** What the preparation is waiting on (mini-player caption). */
    var preparingLabel: String by mutableStateOf("Preparing episode…")
        private set
    /** True while audio comes over the network (the strategy-visible state). */
    var streaming: Boolean by mutableStateOf(false)
        private set
    /** A remote stream's buffer ran dry and the player is rebuffering — the
     *  transports show a spinner instead of a playing icon over silence. */
    var buffering: Boolean by mutableStateOf(false)
        private set
    /** Sticky failure shown in the mini player (tap play to retry). */
    var failureMessage: String? by mutableStateOf(null)
        private set
    var position: Double by mutableStateOf(0.0)
    var rate: Float by mutableStateOf(1.0f)
    /** Sleep timer: counts down ONLY while actually playing (web sleep.rs —
     *  "a paused player holds the timer"); expiry pauses playback once. */
    var sleepRemainingSecs: Double? by mutableStateOf(null)
        private set
    var sleepAtEpisodeEnd: Boolean by mutableStateOf(false)
        private set
    /** Once-per-session latch for `sleepByDefault` auto-arm (web:
     *  sleep_auto_armed) — a manual disable isn't undone by the next episode. */
    private var sleepAutoArmed = false
    /** Set when the minutes timer expires exactly as the episode ends —
     *  suppresses the coincident auto-advance (web: on_ended(!sleep_expired)). */
    private var suppressAdvanceOnEnd = false
    /** Wall-clock millis of the last playing tick (the countdown decrements
     *  by real elapsed time, independent of playback rate). */
    private var lastSleepTickMs: Long? = null

    /** The playlist the user pressed play from (the web's PlayContext):
     *  auto-advance / Up-next / transport Next continue through THIS list;
     *  null = queue semantics. Set by the `context` play entries, preserved
     *  by continuation paths, cleared by stop(). */
    var contextPlaylistId: Int? by mutableStateOf(null)
        private set
    /** Membership snapshot of the context playlist (position order), taken at
     *  play time and kept by the owning screen's optimistic model. */
    private var contextEpisodes: List<EpisodeData> = emptyList()

    /** The now-playing episode's chapter markers (lazily fetched when the row
     *  snapshot lacks them — the web's ensure_episode_chapters). */
    var chapters: List<EpisodeChapterData> by mutableStateOf(emptyList())
        private set

    private var ticksSinceSave = 0
    /** Loading-stall watchdog (web: LOADING_STALL_TICKS ≈ 30s) — a stream
     *  that never reaches ready surfaces an error instead of showing
     *  "playing" forever. */
    private var stallJob: Job? = null
    /** Post-ready dry-buffer guard (see bufferingChanged). */
    private var rebufferJob: Job? = null
    private var tickJob: Job? = null
    /** Late resume target (the detail-fetch path) applied on first READY. */
    private var pendingResume: Double = 0.0
    /** Whether the shared engine currently holds THIS model's media item. */
    private var engineLoaded = false
    private var readyOnce = false
    /** Identity of the current start — a superseded watchdog is a no-op. */
    private var playSession = 0

    /** The ONE shared ExoPlayer (PlaybackService's session player too). */
    private val player: ExoPlayer = PlayerBridge.player(core.appContext)

    private val listener = object : Player.Listener {
        override fun onPlaybackStateChanged(playbackState: Int) {
            if (!engineLoaded) return
            when (playbackState) {
                Player.STATE_READY -> {
                    readyOnce = true
                    val ms = player.duration
                    if (ms != C.TIME_UNSET && ms > 0) duration = ms / 1000.0
                    if (pendingResume > 0) {
                        player.seekTo((pendingResume * 1000).toLong())
                        pendingResume = 0.0
                    }
                }
                Player.STATE_ENDED -> {
                    // Stale-end guard: only the item we actually loaded may
                    // finish the current episode (iOS: ended === currentItem).
                    if (player.currentMediaItem?.mediaId == current?.id?.toString()) {
                        finished()
                    }
                }
                else -> {}
            }
            refreshBuffering()
        }

        /** System pause edges (audio-focus loss, becoming-noisy) arrive here —
         *  persist the cursor and reflect the paused state (iOS interruption/
         *  route-change parity; ExoPlayer handles the focus + resume itself). */
        override fun onPlayWhenReadyChanged(playWhenReady: Boolean, reason: Int) {
            if (!engineLoaded) return
            if (!playWhenReady &&
                (reason == Player.PLAY_WHEN_READY_CHANGE_REASON_AUDIO_FOCUS_LOSS ||
                    reason == Player.PLAY_WHEN_READY_CHANGE_REASON_AUDIO_BECOMING_NOISY)
            ) {
                saveCursorNow()
            }
            if (failureMessage == null) {
                isPlaying = playWhenReady && player.playbackState != Player.STATE_ENDED
            }
            refreshBuffering()
        }

        /** Failure must surface even when ticks never start (a stream that
         *  dies before its first frame). */
        override fun onPlayerError(error: PlaybackException) {
            if (!engineLoaded) return
            isPlaying = false
            buffering = false
            failureMessage = playbackFailureMessage(error)
            healAuthForStreamFailure(error)
            DeviceLog.error("player: item failed: $error")
        }
    }

    init {
        // The newest model owns the shared player; a previous account's
        // model detaches (its registry is already torn down).
        PlayerBridge.adopt(this)
        player.addListener(listener)
    }

    /** Bridge handoff (account switch): stop touching the shared player. */
    fun detach() {
        teardown()
        player.removeListener(listener)
        current = null
        isPlaying = false
        preparing = false
        streaming = false
        failureMessage = null
    }

    // ── controls ─────────────────────────────────────────────────────────────

    /** The continuation playlist a play was launched from (web PlayContext):
     *  its id plus a membership snapshot in position order. */
    data class PlaybackContext(val playlistId: Int, val episodes: List<EpisodeData>)

    /** Continuation play (auto-advance, transport next/prev, retry): keeps
     *  the current play context — the web's context-less request_play. */
    fun play(episode: EpisodeData) {
        start(episode, forceStream = false)
    }

    /** User-entry play from a list (web request_play_in): sets the play
     *  context first; null resets to queue semantics. */
    fun play(episode: EpisodeData, context: PlaybackContext?) {
        setContext(context)
        start(episode, forceStream = false)
    }

    /** Explicit server streaming — the web's "Stream from server" escape
     *  hatch (stream_episode_in): bypasses the device copy and the strategy
     *  tree; a missing server copy is prepared there first. */
    fun stream(episode: EpisodeData, context: PlaybackContext? = null) {
        setContext(context)
        start(episode, forceStream = true)
    }

    private fun setContext(context: PlaybackContext?) {
        contextPlaylistId = context?.playlistId
        contextEpisodes = context?.episodes ?: emptyList()
    }

    /** `forceRestart` rebuilds the pipeline even when `episode` is already
     *  current — the prepare-chain handoffs (server copy ready → replay as
     *  downloaded) must not blip `current` to null, which unmounts the mini
     *  player (and closes an open sheet). */
    private fun start(episode: EpisodeData, forceStream: Boolean, forceRestart: Boolean = false) {
        // A failed item can't be revived by just resuming — a retry must
        // rebuild the source (web: play() from Error routes through
        // request_play's full re-resolution).
        val retrying =
            current?.id == episode.id && (failureMessage != null || player.playerError != null)
        failureMessage = null
        if (!retrying && !forceRestart && current?.id == episode.id && engineLoaded) {
            player.setPlaybackSpeed(rate)
            player.play()
            isPlaying = true
            return
        }
        saveCursorNow()
        teardown()
        current = episode
        // Zero the playhead IMMEDIATELY: the prepare paths below return
        // before the resume cursor is computed, and any saveCursorNow() in
        // that window would persist the PREVIOUS episode's position against
        // this one (web: NowPlaying::preparing zeroes position).
        position = 0.0
        pendingResume = 0.0
        preparing = false
        suppressAdvanceOnEnd = false
        chapters = episode.chapters ?: emptyList()
        if (episode.chapters == null) loadChapters(episode)
        // Auto-arm the sleep timer once per listening session when the pref
        // is set (web: load_and_play → maybe_auto_arm_sleep).
        val prefs = core.models?.prefs?.prefs
        if (prefs?.sleepByDefault == true && !sleepAutoArmed &&
            sleepRemainingSecs == null && !sleepAtEpisodeEnd
        ) {
            setSleepTimer(prefs.defaultSleepMinutes)
            sleepAutoArmed = true
        }

        // Strategy-driven sourcing (the web's PlaybackPreference; embedded is
        // forced to streamOnly). The audio endpoint only serves files the
        // server has stored, so "prepare" first triggers that download.
        // An explicit stream request pins the strategy to streamOnly.
        val strategy =
            if (forceStream) ClientPrefs.PlaybackStrategy.StreamOnly
            else core.effectivePlaybackStrategy
        val hasDevice = core.models?.device?.localFile(episode.id) != null
        val onServer = core.models?.serverDownloads?.isDownloaded(episode)
            ?: (episode.download_status == DownloadStatus.Downloaded)

        when (strategy) {
            ClientPrefs.PlaybackStrategy.StreamOnly -> {
                if (!onServer) {
                    prepareViaServer(episode)
                    return
                }
            }
            ClientPrefs.PlaybackStrategy.StreamFallback -> {
                // Local first; stream only as fallback. Never device-downloads.
                if (!hasDevice && !onServer) {
                    prepareViaServer(episode)
                    return
                }
            }
            ClientPrefs.PlaybackStrategy.StreamFirstAndDownload -> {
                if (!onServer) {
                    prepareViaServer(episode)
                    return
                }
                // Stream now, pull the device copy in the background for later.
                if (!hasDevice && !core.isEmbeddedAccount) {
                    core.models?.device?.download(episode)
                }
            }
            ClientPrefs.PlaybackStrategy.DownloadOnly -> {
                if (!hasDevice) {
                    // Never streams: server copy first (if needed), then the
                    // device copy, then play local bytes.
                    prepareDeviceCopy(episode, needsServer = !onServer)
                    return
                }
            }
        }

        // Prefer the on-device copy (works fully offline) unless the strategy
        // is stream-only; stream otherwise.
        val local =
            if (strategy == ClientPrefs.PlaybackStrategy.StreamOnly) null
            else core.models?.device?.localFile(episode.id)
        val streamUrl = if (local == null) core.audioUrl(episode.id) else null
        if (local == null && streamUrl == null) {
            // No device copy and no usable stream URL (manual offline or
            // signed out) — a silent return here mounted a dead mini player.
            isPlaying = false
            failureMessage =
                if (core.isOffline)
                    "You're offline — download the episode or reconnect to stream it."
                else "Not signed in — can't stream this episode."
            DeviceLog.warn("player: no audio URL for episode ${episode.id}")
            return
        }
        streaming = local == null
        rate = prefs?.defaultRate ?: rate
        // The streaming data source reads the bearer per request.
        PlayerBridge.apiToken = core.apiToken

        val itemBuilder = MediaItem.Builder()
            .setMediaId(episode.id.toString())
            .setMediaMetadata(
                MediaMetadata.Builder()
                    .setTitle(episode.title)
                    .setArtist(episode.podcast?.title ?: "Halogen")
                    .build()
            )
        if (local != null) {
            itemBuilder.setUri(Uri.fromFile(local))
        } else {
            // STREAMS only: the audio URL has no file extension, so hint the
            // type; local device copies carry a real extension.
            itemBuilder.setUri(Uri.parse(streamUrl.toString()))
            itemBuilder.setMimeType(streamMimeHint(episode))
        }

        // Resume from the freshest known cursor (seconds) — the local overlay
        // outranks the row snapshot when newer (offline listening progress
        // must not regress).
        val resume =
            (core.models?.playbacks?.cursor(episode) ?: episode.playback?.cursor ?: 0uL)
                .toLong().toDouble()
        position = resume

        playSession += 1
        engineLoaded = true
        readyOnce = false
        val item = itemBuilder.build()
        if (resume > 0) {
            player.setMediaItem(item, (resume * 1000).toLong())
        } else {
            player.setMediaItem(item)
        }
        player.prepare()
        player.setPlaybackSpeed(rate)
        player.play()
        isPlaying = true

        // Queue/playlist pages can't embed Playback (the server include isn't
        // supported on that route): when neither the overlay nor the row
        // snapshot knows a cursor, ask the episode detail (which does) and
        // late-apply the resume while playback is still at the head.
        if (episode.playback == null && core.models?.playbacks?.entries?.get(episode.id) == null) {
            scope.launch {
                val fresh = runCatching { core.episodeDetail(episode.id) }.getOrNull()
                val cursor = fresh?.playback?.cursor?.toLong() ?: return@launch
                if (cursor <= 0) return@launch
                if (current?.id != episode.id || position >= 5) return@launch
                if (duration > 0) {
                    seek(cursor.toDouble())
                } else {
                    pendingResume = cursor.toDouble()
                    position = pendingResume
                }
            }
        }

        startTicks()
        armLoadingStallGuard(playSession, episode.id)
        PlaybackService.ensureRunning(core.appContext)
        loadArtworkIntoSession(episode)
    }

    /** Web parity: a stream that never reaches ready must not show "playing"
     *  forever (controller.rs LOADING_STALL_TICKS ≈ 30s) — surface a tappable
     *  failure instead. The session/episode identity checks make a superseded
     *  guard a no-op. */
    private fun armLoadingStallGuard(session: Int, episodeId: Int) {
        stallJob?.cancel()
        stallJob = scope.launch {
            delay(30_000)
            if (playSession != session || current?.id != episodeId) return@launch
            if (readyOnce || failureMessage != null) return@launch
            player.pause()
            isPlaying = false
            failureMessage = "Playback timed out — couldn't start this episode."
            DeviceLog.error("player: loading stalled >30s for episode $episodeId")
        }
    }

    private fun refreshBuffering() {
        val waiting =
            engineLoaded && player.playbackState == Player.STATE_BUFFERING && player.playWhenReady
        if (waiting != buffering) {
            buffering = waiting
            bufferingChanged(waiting)
        }
    }

    /** armLoadingStallGuard's shape for AFTER ready: a mid-episode network
     *  drop leaves the buffer dry with `buffering` true forever and no
     *  failure transition — bound it so the spinner becomes a tappable
     *  failure instead of indefinite silence. */
    private fun bufferingChanged(waiting: Boolean) {
        rebufferJob?.cancel()
        rebufferJob = null
        if (!waiting || !streaming) return
        val episodeId = current?.id
        rebufferJob = scope.launch {
            delay(45_000)
            if (current?.id != episodeId || !buffering) return@launch
            player.pause()
            isPlaying = false
            buffering = false
            failureMessage = "Stream stalled — check your connection, then tap play to retry."
            DeviceLog.error("player: rebuffering stalled >45s for episode ${episodeId ?: "?"}")
        }
    }

    /** Walk the cause chain for network/auth facts — a network drop, an
     *  expired stream token, and a broken file need different next steps. */
    private fun streamErrorFacts(error: PlaybackException?): Pair<Boolean, Boolean> {
        var network = false
        var auth = false
        if (error != null) {
            when (error.errorCode) {
                PlaybackException.ERROR_CODE_IO_NETWORK_CONNECTION_FAILED,
                PlaybackException.ERROR_CODE_IO_NETWORK_CONNECTION_TIMEOUT,
                PlaybackException.ERROR_CODE_IO_BAD_HTTP_STATUS -> network = true
            }
            var cursor: Throwable? = error
            while (cursor != null) {
                when (cursor) {
                    is HttpDataSource.InvalidResponseCodeException -> {
                        network = true
                        if (cursor.responseCode == 401 || cursor.responseCode == 403) auth = true
                    }
                    is HttpDataSource.HttpDataSourceException -> network = true
                    is UnknownHostException, is SocketTimeoutException,
                    is java.net.ConnectException -> network = true
                }
                cursor = cursor.cause
            }
        }
        return network to auth
    }

    /** The audio stream bypasses the client's 401 machinery (the bearer is
     *  attached by the data source, not the API client) — on an auth-shaped
     *  stream failure, probe an authenticated endpoint so the shared token
     *  box refreshes; the retry then rebuilds the source with a fresh token. */
    private fun healAuthForStreamFailure(error: PlaybackException?) {
        val episode = current ?: return
        if (!streaming || !streamErrorFacts(error).second) return
        scope.launch { runCatching { core.episodeDetail(episode.id) } }
    }

    private fun playbackFailureMessage(error: PlaybackException?): String {
        val (sawNetwork, sawAuth) = streamErrorFacts(error)
        if (streaming) {
            if (sawAuth) {
                return "The stream was refused — tap play to retry, or sign in again."
            }
            if (sawNetwork || core.isOffline) {
                return "Stream interrupted — check your connection, then tap play to retry."
            }
            return "Couldn't play this stream — tap play to retry, or download the episode."
        }
        return "Couldn't play the downloaded file — it may be damaged. Remove local data and re-download it."
    }

    /** Lazy chapter fetch when the row snapshot has none embedded: cached
     *  detail first (offline), then the network detail (which includes
     *  Chapters) — the web's ensure_episode_chapters. */
    private fun loadChapters(episode: EpisodeData) {
        scope.launch {
            var detail = accountStore?.load<EpisodeData>(CacheKey.episode(episode.id))
            if (detail?.chapters == null) {
                detail = runCatching { core.episodeDetail(episode.id) }.getOrNull() ?: detail
            }
            val found = detail?.chapters ?: return@launch
            if (current?.id != episode.id) return@launch
            chapters = found
        }
    }

    /** The chapter currently playing — the last marker at or before the
     *  position (web now_playing.rs active_chapter_index). */
    val activeChapterIndex: Int?
        get() = chapters.indexOfLast { it.starts_at_secs <= position }.takeIf { it >= 0 }

    fun toggle() {
        // From Error a bare resume can't revive the failed item — route
        // through the full source rebuild (web: toggle and the OS play
        // command both go through play() → request_play for exactly this).
        val cur = current
        if (cur != null && (failureMessage != null || player.playerError != null)) {
            start(cur, forceStream = false)
            return
        }
        if (!engineLoaded) return
        if (isPlaying) {
            player.pause()
            isPlaying = false
            saveCursorNow()
        } else {
            player.setPlaybackSpeed(rate)
            player.play()
            isPlaying = true
            // Resuming after a sleep-timer pause starts a fresh listening
            // stretch — the expired timer must not eat the next episode end.
            suppressAdvanceOnEnd = false
            lastSleepTickMs = null
        }
    }

    fun seek(seconds: Double) {
        // A seek supersedes any queued resume cursor — the READY handler must
        // not snap a pre-ready chapter-jump/skip back to the saved position.
        pendingResume = 0.0
        position = seconds
        if (engineLoaded) player.seekTo((seconds * 1000).toLong())
        saveCursorNow()
    }

    fun skip(delta: Double) {
        seek(max(0.0, position + delta))
    }

    /** Prefs-configured jump distances (Settings → Playback). */
    val skipForwardSecs: Double
        get() = (core.models?.prefs?.prefs?.skipForwardSecs ?: 30).toDouble()

    val skipBackSecs: Double
        get() = (core.models?.prefs?.prefs?.skipBackSecs ?: 15).toDouble()

    fun applyRate(new: Float) {
        rate = new
        if (isPlaying) player.setPlaybackSpeed(new)
        // Persist as the new default so the choice sticks across tracks —
        // play() applies `defaultRate` on every load (web: the now-playing
        // speed picker writes playback_rate to config).
        if (core.models?.prefs?.prefs?.defaultRate != new) {
            core.models?.prefs?.update { it.copy(defaultRate = new) }
        }
    }

    /** null cancels; 0 = end of episode; otherwise a countdown of `minutes`
     *  of PLAYING time (web sleep.rs: only ticks down while playing). */
    fun setSleepTimer(minutes: Int?) {
        sleepRemainingSecs = null
        sleepAtEpisodeEnd = false
        lastSleepTickMs = null
        if (minutes == null) return
        if (minutes == 0) {
            sleepAtEpisodeEnd = true
            return
        }
        sleepRemainingSecs = (minutes * 60).toDouble()
    }

    /** Whole minutes remaining, rounded up (web SleepState::remaining_minutes
     *  — the badge reads "1" until the timer truly reaches zero). */
    val sleepRemainingMinutes: Int?
        get() = sleepRemainingSecs?.let { ceil(it / 60.0).toInt() }

    /** Count the timer down by real elapsed wall time — called from tick().
     *  Pauses once on the expiry edge and remembers it so a coincident
     *  episode end doesn't auto-advance (web: tick's sleep_expired
     *  suppression). */
    private fun advanceSleepTimer() {
        val remaining = sleepRemainingSecs
        if (!isPlaying || remaining == null) {
            lastSleepTickMs = null
            return
        }
        val now = System.currentTimeMillis()
        // Ticks are ~1s of playback apart; clamp the wall delta so a
        // backgrounded/suspended stretch can't burn the whole timer at once.
        val dt = lastSleepTickMs?.let { min(max((now - it) / 1000.0, 0.0), 5.0) } ?: 1.0
        lastSleepTickMs = now
        val next = remaining - dt
        if (next <= 0) {
            sleepRemainingSecs = null
            lastSleepTickMs = null
            suppressAdvanceOnEnd = true
            toggle()  // pause + cursor save
        } else {
            sleepRemainingSecs = next
        }
    }

    fun stop() {
        saveCursorNow()
        teardown()
        current = null
        chapters = emptyList()
        isPlaying = false
        preparing = false
        streaming = false
        // Closing the player ends the listening session: clear the play
        // context and sleep timer, re-arm the auto-arm latch (web: stop()).
        contextPlaylistId = null
        contextEpisodes = emptyList()
        setSleepTimer(null)
        sleepAutoArmed = false
        suppressAdvanceOnEnd = false
    }

    /** downloadOnly: chain server copy (when missing) → device copy → play
     *  the local bytes. Progress captions track each stage. */
    private fun prepareDeviceCopy(episode: EpisodeData, needsServer: Boolean) {
        preparing = true
        preparingLabel = if (needsServer) "Preparing on server…" else "Downloading to device…"
        if (needsServer) {
            core.models?.serverDownloads?.download(episode)
        } else {
            core.models?.device?.download(episode)
        }
        scope.launch {
            var serverDone = !needsServer
            for (i in 0 until 900) {
                delay(1_000)
                if (current?.id != episode.id) return@launch
                if (!serverDone) {
                    if (core.models?.serverDownloads?.failed?.contains(episode.id) == true) {
                        preparing = false
                        failureMessage = "Server download failed"
                        return@launch
                    }
                    if (core.models?.serverDownloads?.completed?.contains(episode.id) == true) {
                        serverDone = true
                        preparingLabel = "Downloading to device…"
                        core.models?.device?.download(markedDownloaded(episode))
                    }
                    continue
                }
                when (val state = core.models?.device?.stateOf(episode.id)
                    ?: DeviceDownloads.State.None) {
                    is DeviceDownloads.State.Downloaded -> {
                        preparing = false
                        // forceRestart, NOT current=null+play: a null blip
                        // unmounts the mini player and kills the open sheet.
                        start(markedDownloaded(episode), forceStream = false, forceRestart = true)
                        return@launch
                    }
                    is DeviceDownloads.State.Paused -> {
                        preparing = false
                        failureMessage = "Device download paused (tap to resume)"
                        return@launch
                    }
                    is DeviceDownloads.State.Failed -> {
                        preparing = false
                        failureMessage = state.message
                        return@launch
                    }
                    is DeviceDownloads.State.None ->
                        core.models?.device?.download(markedDownloaded(episode))
                    is DeviceDownloads.State.Downloading,
                    is DeviceDownloads.State.WaitingServer -> {}
                }
            }
            preparing = false
            failureMessage = "Timed out preparing the episode"
        }
    }

    /** Un-downloaded episode: kick the server download, mirror its progress
     *  in the mini player, then start streaming when it completes. */
    private fun prepareViaServer(episode: EpisodeData) {
        preparing = true
        preparingLabel = "Preparing on server…"
        core.models?.serverDownloads?.download(episode)
        scope.launch {
            for (i in 0 until 450) {
                delay(1_000)
                if (current?.id != episode.id) return@launch
                if (core.models?.serverDownloads?.failed?.contains(episode.id) == true) {
                    preparing = false
                    failureMessage = "Server download failed"
                    return@launch
                }
                if (core.models?.serverDownloads?.completed?.contains(episode.id) == true) {
                    preparing = false
                    // forceRestart, NOT current=null+play (see start()).
                    start(markedDownloaded(episode), forceStream = false, forceRestart = true)
                    return@launch
                }
            }
            preparing = false
            failureMessage = "Timed out preparing the episode"
        }
    }

    /** Copy with download_status flipped so the decision tree streams. */
    private fun markedDownloaded(e: EpisodeData): EpisodeData =
        e.copy(download_status = DownloadStatus.Downloaded)

    /** MIME hint for a server stream, from the extension of the server copy's
     *  stored file (preserved from the RSS origin by the download pipeline),
     *  falling back to the enclosure URL, then to audio/mpeg (the dominant
     *  podcast type). */
    private fun streamMimeHint(episode: EpisodeData): String {
        val path = episode.content_file_path ?: episode.content_url
        val name = path.substringBefore('?').substringBefore('#').substringAfterLast('/')
        return when (name.substringAfterLast('.', "").lowercase()) {
            "m4a", "mp4" -> "audio/mp4"
            "aac" -> "audio/aac"
            "ogg", "oga" -> "audio/ogg"
            "opus" -> "audio/opus"
            "wav" -> "audio/wav"
            "flac" -> "audio/flac"
            else -> "audio/mpeg"
        }
    }

    // ── internals ────────────────────────────────────────────────────────────

    private fun startTicks() {
        tickJob?.cancel()
        tickJob = scope.launch {
            while (isActive) {
                delay(1_000)
                tick()
            }
        }
    }

    private fun tick() {
        if (!engineLoaded) return
        position = player.currentPosition.coerceAtLeast(0) / 1000.0
        val ms = player.duration
        if (ms != C.TIME_UNSET && ms > 0) duration = ms / 1000.0
        // Truth-sync: a stalled/failed item must not show a pause icon
        // (failures themselves land through onPlayerError).
        if (failureMessage == null && player.playerError == null) {
            isPlaying = player.playWhenReady && player.playbackState != Player.STATE_ENDED
        }
        // Sleep countdown rides the playback ticks — a paused player holds
        // the timer (web: persist_while_playing → advance_sleep).
        advanceSleepTimer()
        if (player.isPlaying) {
            ticksSinceSave += 1
            // Persist roughly every 10s of playback (coalesced in the outbox).
            if (ticksSinceSave >= 10) {
                ticksSinceSave = 0
                saveCursorNow()
            }
        }
    }

    /** End of episode: mark finished, then auto-advance to the item AFTER the
     *  current one in the continuation playlist (the play context, else the
     *  queue) when the setting allows — the web's on_ended. The queue is
     *  never mutated on completion (the web doesn't remove finished items). */
    private fun finished() {
        val episode = current ?: return
        // Optimistic overlay + outbox: lists/menus/History flip immediately.
        core.models?.playbacks?.markPlayed(episode, true)
        // markPlayed wrote the terminal cursor: zero `position` BEFORE stop()/play()
        // so their saveCursorNow() can't re-persist the end position. `current` stays
        // set — nil-ing it blips the mini player away and dismisses an open sheet.
        position = 0.0
        if (sleepAtEpisodeEnd || suppressAdvanceOnEnd) {
            // The user asked to stop here — that wins over the continuation
            // (web: on_ended(!sleep_expired)).
            suppressAdvanceOnEnd = false
            setSleepTimer(null)
            stop()
            return
        }
        val autoAdvance = core.models?.prefs?.prefs?.autoAdvance ?: true
        val next = nextUp(episode)
        if (autoAdvance && next != null) {
            play(next)
        } else {
            stop()
        }
    }

    // ── continuation (play context → queue), web navigation.rs ───────────────

    /** Continue through live play-context membership, or the queue when no context is set.
     * Resolve episode objects from the play-time snapshot and queue; skip unknown additions. */
    private fun continuationList(): List<EpisodeData> {
        val ctxId = contextPlaylistId ?: return core.models?.queue?.episodes ?: emptyList()
        val ids = core.models?.playlists?.playlists
            ?.firstOrNull { it.id == ctxId }?.episode_ids
            ?: return contextEpisodes
        val byId = mutableMapOf<Int, EpisodeData>()
        for (episode in core.models?.queue?.episodes ?: emptyList()) byId[episode.id] = episode
        for (episode in contextEpisodes) byId[episode.id] = episode
        return ids.mapNotNull { byId[it] }
    }

    /** The episode that should play next after `episode` (web
     *  PlaylistState::next_up_in): its position-neighbor when it's in the
     *  continuation list (null past the end — playback stops at the tail),
     *  else the list's head. */
    private fun nextUp(after: EpisodeData): EpisodeData? {
        val list = continuationList()
        val idx = list.indexOfFirst { it.id == after.id }
        if (idx >= 0) return if (idx + 1 < list.size) list[idx + 1] else null
        return list.firstOrNull()
    }

    /** The episode `delta` (±1) places from the current one in the active
     *  list: the context playlist when the current episode is in it, else the
     *  queue (web navigation::adjacent_episode; the podcast-order fallback is
     *  omitted — no full podcast episode list is held in memory). */
    private fun adjacentEpisode(delta: Int): EpisodeData? {
        val cur = current ?: return null
        val lists = buildList {
            if (contextPlaylistId != null) add(continuationList())
            add(core.models?.queue?.episodes ?: emptyList())
        }
        for (list in lists) {
            val idx = list.indexOfFirst { it.id == cur.id }
            if (idx < 0) continue
            val target = idx + delta
            return if (target in list.indices) list[target] else null
        }
        return null
    }

    /** Transport Next target: the continuation neighbor, else null. */
    private fun nextEpisodeTarget(): EpisodeData? = current?.let { nextUp(it) }

    /** The "Up next" preview target (web UpNext over next_up_in). */
    val upNext: EpisodeData? get() = nextEpisodeTarget()

    val hasNext: Boolean get() = nextEpisodeTarget() != null
    val hasPrevious: Boolean get() = adjacentEpisode(-1) != null

    /** UI/media-session transport: jump to the next episode (continuation
     *  playlist → queue). Preserves the play context (web play_next_episode). */
    fun playNextEpisode() {
        play(nextEpisodeTarget() ?: return)
    }

    /** UI/media-session transport: jump to the previous episode. */
    fun playPreviousEpisode() {
        play(adjacentEpisode(-1) ?: return)
    }

    // ── media session bridge (PlaybackService routes commands here) ──────────

    fun remotePlay() {
        if (!isPlaying) toggle()
    }

    fun remotePause() {
        if (isPlaying) toggle()
    }

    fun remoteSeekTo(seconds: Double) = seek(seconds)

    /** Next/previous episode — with mediaNextPrevSeek set they SEEK instead:
     *  Bluetooth devices without dedicated seek buttons send track commands
     *  (web parity: media_session.rs's media_next_prev_seek branch). */
    fun remoteNext() {
        if (mediaNextPrevSeek) skip(skipForwardSecs) else playNextEpisode()
    }

    fun remotePrevious() {
        if (mediaNextPrevSeek) skip(-skipBackSecs) else playPreviousEpisode()
    }

    /** Seek-override mode: track commands work whenever something is loaded;
     *  otherwise they follow the continuation list's ends. */
    val remoteNextEnabled: Boolean
        get() = if (mediaNextPrevSeek) current != null else hasNext

    val remotePreviousEnabled: Boolean
        get() = if (mediaNextPrevSeek) current != null else hasPrevious

    /** Bluetooth next/prev-as-seek override (web media_next_prev_seek). */
    private val mediaNextPrevSeek: Boolean
        get() = core.models?.prefs?.prefs?.mediaNextPrevSeek ?: false

    private fun saveCursorNow() {
        // Never persist while preparing: nothing has played yet, and the
        // position field is not this episode's playhead (web persist_cursor
        // refuses in Preparing for the same reason).
        val episode = current ?: return
        if (position <= 0 || preparing) return
        val cursor = max(0.0, position).toLong().toULong()
        // Overlay first (so every list/detail/relaunch resumes here), then
        // the coalescing outbox op — the web's SetCursor command in order.
        core.models?.playbacks?.setCursor(episode, cursor)
    }

    private fun teardown() {
        stallJob?.cancel()
        stallJob = null
        rebufferJob?.cancel()
        rebufferJob = null
        tickJob?.cancel()
        tickJob = null
        ticksSinceSave = 0
        pendingResume = 0.0
        // Flip the flag FIRST so listener callbacks from stop/clear no-op.
        val hadItem = engineLoaded
        engineLoaded = false
        readyOnce = false
        buffering = false
        if (hadItem) {
            player.stop()
            player.clearMediaItems()
        }
        duration = 0.0
    }

    /** Fetch the full-size art and attach it to the session's media item so
     *  the notification/lock screen shows it (iOS MPNowPlaying artwork). */
    private fun loadArtworkIntoSession(episode: EpisodeData) {
        val url = core.episodeArtUrl(episode, small = false) ?: return
        val session = playSession
        scope.launch {
            val bytes = runCatching {
                val loader = ArtLoader.loader(core.appContext)
                val request = ImageRequest.Builder(core.appContext).data(url).build()
                val image = (loader.execute(request) as? SuccessResult)?.image
                    ?: return@runCatching null
                withContext(Dispatchers.IO) {
                    val bitmap = image.toBitmap()
                    ByteArrayOutputStream().use { out ->
                        bitmap.compress(Bitmap.CompressFormat.PNG, 100, out)
                        out.toByteArray()
                    }
                }
            }.getOrNull() ?: return@launch
            if (playSession != session || !engineLoaded) return@launch
            if (current?.id != episode.id) return@launch
            val item = player.currentMediaItem ?: return@launch
            val metadata = item.mediaMetadata.buildUpon()
                .setArtworkData(bytes, MediaMetadata.PICTURE_TYPE_FRONT_COVER)
                .build()
            // Same localConfiguration → metadata-only replace, no interruption.
            player.replaceMediaItem(
                player.currentMediaItemIndex,
                item.buildUpon().setMediaMetadata(metadata).build(),
            )
        }
    }
}
