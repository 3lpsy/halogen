package org.fgsec.halogen.player

import android.content.Context
import android.content.Intent
import androidx.annotation.OptIn
import androidx.core.content.ContextCompat
import androidx.media3.common.AudioAttributes
import androidx.media3.common.C
import androidx.media3.common.ForwardingPlayer
import androidx.media3.common.Player
import androidx.media3.common.util.UnstableApi
import androidx.media3.datasource.DefaultDataSource
import androidx.media3.datasource.okhttp.OkHttpDataSource
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.exoplayer.source.DefaultMediaSourceFactory
import androidx.media3.session.MediaSession
import androidx.media3.session.MediaSessionService
import okhttp3.Call
import org.fgsec.halogen.features.player.PlayerModel
import org.fgsec.halogen.networking.Http

/// The ONE ExoPlayer shared between the app (PlayerModel drives it) and the
/// MediaSessionService (media notification + external controllers) — both
/// sides see the same instance, so state and transport never fork.
object PlayerBridge {
    /** The active account's PlayerModel — session commands route through it. */
    @Volatile
    var model: PlayerModel? = null
        private set

    /** Bearer for the streaming data source (set before every prepare). */
    @Volatile
    var apiToken: String? = null

    private var shared: ExoPlayer? = null

    /** The newest model owns the player; the previous account's detaches. */
    fun adopt(next: PlayerModel) {
        val previous = model
        model = null
        previous?.detach()
        model = next
    }

    /** Lazily build the shared player (main thread: model init / service
     *  onCreate). Streams ride the shared OkHttp client (cookie jar) with the
     *  API bearer injected per request; file URIs play the device copies. */
    @OptIn(UnstableApi::class)
    fun player(context: Context): ExoPlayer {
        shared?.let { return it }
        val app = context.applicationContext
        val http = OkHttpDataSource.Factory(
            Call.Factory { request ->
                val token = apiToken
                val authed =
                    if (token != null)
                        request.newBuilder().header("Authorization", "Bearer $token").build()
                    else request
                Http.client.newCall(authed)
            }
        )
        val player = ExoPlayer.Builder(app)
            .setMediaSourceFactory(DefaultMediaSourceFactory(DefaultDataSource.Factory(app, http)))
            // Audio-focus pause/resume + headphone-unplug pause — the iOS
            // interruption/route-change parity (AudioFocusRequest + the
            // becoming-noisy receiver live inside ExoPlayer).
            .setAudioAttributes(
                AudioAttributes.Builder()
                    .setUsage(C.USAGE_MEDIA)
                    .setContentType(C.AUDIO_CONTENT_TYPE_SPEECH)
                    .build(),
                /* handleAudioFocus = */ true,
            )
            .setHandleAudioBecomingNoisy(true)
            .setWakeMode(C.WAKE_MODE_NETWORK)
            .build()
        shared = player
        return player
    }
}

/// Foreground media service: keeps the process (and the embedded server)
/// alive during playback and surfaces the media notification. The session
/// player is the bridge's shared ExoPlayer behind a routing wrapper.
class PlaybackService : MediaSessionService() {
    private var session: MediaSession? = null

    override fun onCreate() {
        super.onCreate()
        session = MediaSession.Builder(this, BridgedPlayer(PlayerBridge.player(this))).build()
    }

    override fun onGetSession(controllerInfo: MediaSession.ControllerInfo): MediaSession? = session

    override fun onTaskRemoved(rootIntent: Intent?) {
        // App swiped away while paused/empty: nothing to keep alive.
        val player = session?.player
        if (player == null || !player.playWhenReady || player.mediaItemCount == 0) {
            stopSelf()
        }
    }

    override fun onDestroy() {
        // The player is shared with the app — release the session only.
        session?.release()
        session = null
        super.onDestroy()
    }

    companion object {
        /** Started right before playback so the notification can attach. */
        fun ensureRunning(context: Context) {
            runCatching {
                ContextCompat.startForegroundService(
                    context, Intent(context, PlaybackService::class.java)
                )
            }
        }
    }
}

/// Routes notification/session transport onto the PlayerModel: play from a
/// failed item retries through the full source rebuild, next/prev honor
/// mediaNextPrevSeek, and seek increments follow the Settings skip intervals
/// (iOS MPRemoteCommandCenter parity).
private class BridgedPlayer(player: Player) : ForwardingPlayer(player) {
    private val model: PlayerModel? get() = PlayerBridge.model

    override fun play() {
        model?.remotePlay() ?: super.play()
    }

    override fun pause() {
        model?.remotePause() ?: super.pause()
    }

    override fun seekToNext() {
        model?.remoteNext() ?: super.seekToNext()
    }

    override fun seekToNextMediaItem() {
        model?.remoteNext() ?: super.seekToNextMediaItem()
    }

    override fun seekToPrevious() {
        model?.remotePrevious() ?: super.seekToPrevious()
    }

    override fun seekToPreviousMediaItem() {
        model?.remotePrevious() ?: super.seekToPreviousMediaItem()
    }

    override fun seekForward() {
        model?.let { it.skip(it.skipForwardSecs) } ?: super.seekForward()
    }

    override fun seekBack() {
        model?.let { it.skip(-it.skipBackSecs) } ?: super.seekBack()
    }

    override fun getSeekForwardIncrement(): Long =
        model?.let { (it.skipForwardSecs * 1000).toLong() } ?: super.getSeekForwardIncrement()

    override fun getSeekBackIncrement(): Long =
        model?.let { (it.skipBackSecs * 1000).toLong() } ?: super.getSeekBackIncrement()

    override fun seekTo(positionMs: Long) {
        model?.remoteSeekTo(positionMs / 1000.0) ?: super.seekTo(positionMs)
    }

    override fun getAvailableCommands(): Player.Commands {
        val m = model ?: return super.getAvailableCommands()
        val builder = super.getAvailableCommands().buildUpon()
            .add(Player.COMMAND_SEEK_BACK)
            .add(Player.COMMAND_SEEK_FORWARD)
        if (m.remoteNextEnabled) builder.add(Player.COMMAND_SEEK_TO_NEXT)
        else builder.remove(Player.COMMAND_SEEK_TO_NEXT)
        if (m.remotePreviousEnabled) builder.add(Player.COMMAND_SEEK_TO_PREVIOUS)
        else builder.remove(Player.COMMAND_SEEK_TO_PREVIOUS)
        return builder.build()
    }

    override fun isCommandAvailable(command: Int): Boolean =
        getAvailableCommands().contains(command)
}
