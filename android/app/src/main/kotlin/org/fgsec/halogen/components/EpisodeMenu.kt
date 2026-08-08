package org.fgsec.halogen.components

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.size
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.unit.dp
import kotlin.math.max
import kotlinx.coroutines.launch
import org.fgsec.halogen.core.ClientPrefs
import org.fgsec.halogen.core.DeviceDownloads
import org.fgsec.halogen.core.HalogenCore
import org.fgsec.halogen.features.player.PlayerModel
import org.fgsec.halogen.features.playlists.PlaylistEpisodesModel
import org.fgsec.halogen.storage.CacheKey
import org.fgsec.halogen.storage.OutboxOp
import org.fgsec.halogen.wire.DownloadStatus
import org.fgsec.halogen.wire.EpisodeData
import org.fgsec.halogen.wire.PlaybackStatus

/// Where an episode row is rendered — decides which quick actions its menu
/// offers (the web's per-page row context menus).
sealed class EpisodeMenuContext {
    /// Browsing lists (Latest / podcast episodes / History / Downloads).
    data object Browse : EpisodeMenuContext()

    /// The queue: adds move-to-top + remove-from-queue.
    data object Queue : EpisodeMenuContext()

    /// A playlist detail: adds remove-from-<name>. Carries the screen's model
    /// so the remove updates the visible list + cache (same path as the swipe).
    data class Playlist(
        val name: String,
        val model: PlaylistEpisodesModel,
    ) : EpisodeMenuContext()

    /// The play context this row's list contributes (web: rows call
    /// request_play_in with their playlist_id): a playlist detail continues
    /// through that playlist; browse lists and the queue reset to queue
    /// semantics (null ≡ the web's None).
    val playbackContext: PlayerModel.PlaybackContext?
        get() = (this as? Playlist)?.let {
            PlayerModel.PlaybackContext(
                playlistId = it.model.playlistId, episodes = it.model.episodes)
        }
}

/// One row of a DropdownMenu, iOS Label shape (destructive = red).
@Composable
fun EpisodeMenuItem(
    label: String,
    sf: String,
    destructive: Boolean = false,
    onClick: () -> Unit,
) {
    val text =
        if (destructive) MaterialTheme.colorScheme.error
        else MaterialTheme.colorScheme.onSurface
    val icon =
        if (destructive) MaterialTheme.colorScheme.error
        else MaterialTheme.colorScheme.onSurfaceVariant
    DropdownMenuItem(
        text = { Text(label, color = text) },
        leadingIcon = {
            Icon(
                halogenIcon(sf),
                contentDescription = null,
                tint = icon,
                modifier = Modifier.size(20.dp),
            )
        },
        onClick = onClick,
    )
}

/// The quick-action menu content for an episode row — shared by the visible
/// ellipsis button and the long-press context menu so both always agree.
/// Render inside a DropdownMenu; `dismiss` closes it after an action.
@Composable
fun EpisodeMenu(
    episode: EpisodeData,
    context: EpisodeMenuContext,
    core: HalogenCore,
    dismiss: () -> Unit,
) {
    val models = core.models
    val navigator = LocalNavigator.current
    var playlistsOpen by remember { mutableStateOf(false) }

    val onServer = models?.serverDownloads?.isDownloaded(episode)
        ?: (episode.download_status == DownloadStatus.Downloaded)

    // The "Manage Playlists" submenu page (iOS nested Menu): membership-aware
    // toggles (web episode_playlists.rs) — checkmark when already a member,
    // tap toggles add ↔ remove. Both directions are optimistic outbox ops +
    // cache patches, so the menu and the target list flip offline too.
    if (playlistsOpen) {
        EpisodeMenuItem("Manage Playlists", "chevron.left") { playlistsOpen = false }
        val playlists = models?.playlists
        if (playlists != null) {
            for (playlist in playlists.playlists.filter { !it.is_default }) {
                val member = playlist.episode_ids?.contains(episode.id) == true
                DropdownMenuItem(
                    text = { Text(playlist.name) },
                    leadingIcon = if (member) {
                        {
                            Icon(
                                halogenIcon("checkmark"),
                                contentDescription = "In playlist",
                                tint = MaterialTheme.colorScheme.onSurfaceVariant,
                                modifier = Modifier.size(20.dp),
                            )
                        }
                    } else null,
                    onClick = {
                        if (member) playlists.remove(episode, playlist)
                        else playlists.add(episode, playlist)
                        dismiss()
                    },
                )
            }
        }
        return
    }

    EpisodeMenuItem("Play", "play.fill") {
        models?.player?.play(episode, context.playbackContext)
        dismiss()
    }

    // The explicit "Stream from server" escape hatch — shown when the server
    // holds a copy, this device doesn't, and the strategy doesn't forbid
    // streaming (web episode_menu.rs gates; embedded Play already IS local
    // streaming).
    val onDevice =
        models?.device?.stateOf(episode.id) == DeviceDownloads.State.Downloaded
    val strategy =
        models?.prefs?.prefs?.playbackStrategy ?: ClientPrefs.PlaybackStrategy.DownloadOnly
    if (!core.isEmbeddedAccount && strategy != ClientPrefs.PlaybackStrategy.DownloadOnly &&
        onServer && !onDevice && !core.isOffline
    ) {
        EpisodeMenuItem("Stream from server", "dot.radiowaves.left.and.right") {
            models?.player?.stream(episode, context.playbackContext)
            dismiss()
        }
    }

    // Queue section.
    val queue = models?.queue
    if (queue != null) {
        when {
            context is EpisodeMenuContext.Queue -> EpisodeMenuItem(
                "Move to top", "arrow.up.to.line",
            ) {
                queue.moveToTop(episode)
                dismiss()
            }
            queue.contains(episode) -> EpisodeMenuItem(
                "Remove from Queue", "minus.circle",
            ) {
                queue.remove(episode)
                dismiss()
            }
            else -> EpisodeMenuItem("Add to Queue", "text.badge.plus") {
                queue.add(episode)
                dismiss()
            }
        }
    }

    // Not plain "Playlists" — that label collides with the tab bar button in
    // the accessibility tree (ambiguous UI tests).
    if (models?.playlists != null) {
        EpisodeMenuItem("Manage Playlists", "music.note.list") { playlistsOpen = true }
    }

    // Overlay-wins: an offline toggle must flip this menu immediately, not
    // keep offering the same action off a stale row snapshot.
    val status = models?.playbacks?.status(episode)
        ?: episode.playback_status ?: PlaybackStatus.Unplayed
    if (status == PlaybackStatus.Finished) {
        EpisodeMenuItem("Mark unplayed", "circle") {
            models?.playbacks?.markPlayed(episode, false)
            dismiss()
        }
    } else {
        EpisodeMenuItem("Mark played", "checkmark.circle") {
            models?.playbacks?.markPlayed(episode, true)
            dismiss()
        }
    }

    // Device section — embedded accounts never device-download.
    val device = models?.device
    if (device != null && !core.isEmbeddedAccount) {
        when (device.stateOf(episode.id)) {
            is DeviceDownloads.State.Downloaded,
            is DeviceDownloads.State.Downloading,
            is DeviceDownloads.State.Paused,
            is DeviceDownloads.State.WaitingServer -> EpisodeMenuItem(
                "Remove from device", "iphone.slash", destructive = true,
            ) {
                device.remove(episode.id)
                dismiss()
            }
            is DeviceDownloads.State.Failed -> {
                // Retry (resumes the durable partial) or clear the traces.
                EpisodeMenuItem("Retry download to device", "arrow.down.to.line.circle") {
                    device.download(episode)
                    dismiss()
                }
                EpisodeMenuItem("Remove from device", "iphone.slash", destructive = true) {
                    device.remove(episode.id)
                    dismiss()
                }
            }
            is DeviceDownloads.State.None -> EpisodeMenuItem(
                "Download to device", "arrow.down.to.line.circle",
            ) {
                device.download(episode)
                dismiss()
            }
        }
    }

    // Server section.
    val embedded = core.isEmbeddedAccount
    if (onServer) {
        EpisodeMenuItem(
            if (embedded) "Re-download" else "Re-download on server",
            "arrow.triangle.2.circlepath",
        ) {
            // Force fresh: remove drains first, then the trigger re-fetches
            // (web: RedownloadOnServer's op order).
            core.scope.launch {
                core.outbox?.enqueue(OutboxOp.Kind.RemoveServerDownload(episode.id))
                models?.serverDownloads?.download(episode)
            }
            dismiss()
        }
        EpisodeMenuItem(
            if (embedded) "Remove download" else "Remove from server",
            if (embedded) "trash" else "icloud.slash",
            destructive = true,
        ) {
            // Optimistic overlay first (rows/menus flip immediately — web
            // resets the status before enqueueing), then the durable op
            // (queues offline instead of silently dropping).
            models?.serverDownloads?.markRemovedLocally(episode.id)
            core.scope.launch {
                core.outbox?.enqueue(OutboxOp.Kind.RemoveServerDownload(episode.id))
            }
            dismiss()
        }
    } else if (episode.download_status != DownloadStatus.Downloading) {
        EpisodeMenuItem(
            if (embedded) "Download" else "Download on server",
            if (embedded) "arrow.down.circle" else "icloud.and.arrow.down",
        ) {
            models?.serverDownloads?.download(episode)
            dismiss()
        }
    }

    // "View podcast" — every row menu offers it on the web; the detail page
    // reuses this same menu, so it rides along there too.
    EpisodeMenuItem("View podcast", "square.grid.2x2") {
        navigator?.push(AppRoute.Podcast(episode.podcast_id))
        dismiss()
    }

    // Context section.
    when (context) {
        is EpisodeMenuContext.Queue -> EpisodeMenuItem(
            "Remove from Queue", "minus.circle", destructive = true,
        ) {
            models?.queue?.remove(episode)
            dismiss()
        }
        is EpisodeMenuContext.Playlist -> EpisodeMenuItem(
            "Remove from ${context.name}", "minus.circle", destructive = true,
        ) {
            // Same path as the swipe action: model.remove updates the visible
            // list + cache AND enqueues the op.
            context.model.remove(episode)
            dismiss()
        }
        is EpisodeMenuContext.Browse -> {}
    }

    // "Remove local data" — wipe THIS episode's local traces (device bytes,
    // cached detail, overlay playback) without touching the server; it
    // re-syncs on the next pull (web: confirm_purge).
    EpisodeMenuItem("Remove local data", "arrow.counterclockwise", destructive = true) {
        models?.device?.remove(episode.id)
        models?.playbacks?.purge(episode.id)
        core.scope.launch { core.store?.remove(CacheKey.episode(episode.id)) }
        // Menus can't host a confirm dialog; at minimum the destructive wipe
        // must acknowledge itself.
        ToastCenter.success("Removed this episode's local data")
        dismiss()
    }
}

/// The row's download control, mirroring the web's tiered icons: on device →
/// trash; downloading → progress ring (download glyph pauses/resumes, CLOUD glyph
/// tracks the server); on server only → cloud-pull; nowhere → full-chain download.
@Composable
fun DownloadButton(episode: EpisodeData, core: HalogenCore) {
    val models = core.models
    // Embedded accounts never device-download (stream-only; the media already
    // lives in the on-device server) — the button manages the SERVER copy there.
    val device: DeviceDownloads.State =
        if (core.isEmbeddedAccount) DeviceDownloads.State.None
        else models?.device?.stateOf(episode.id) ?: DeviceDownloads.State.None
    val serverProgress = models?.serverDownloads?.progress(episode.id)
    val onServer = models?.serverDownloads?.isDownloaded(episode)
        ?: (episode.download_status == DownloadStatus.Downloaded)
    val serverRunning =
        serverProgress != null || episode.download_status == DownloadStatus.Downloading
    val serverFailed = models?.serverDownloads?.failed?.contains(episode.id) == true

    // A row already DOWNLOADING server-side gets live tracking.
    LaunchedEffect(episode.id, episode.download_status) {
        if (device == DeviceDownloads.State.None &&
            episode.download_status == DownloadStatus.Downloading
        ) {
            core.models?.serverDownloads?.watch(episode.id)
        }
    }

    Box(
        Modifier
            .size(width = 30.dp, height = 32.dp)
            .clickable {
                when (device) {
                    is DeviceDownloads.State.Downloaded ->
                        models?.device?.remove(episode.id)
                    is DeviceDownloads.State.Downloading,
                    is DeviceDownloads.State.WaitingServer ->
                        models?.device?.pause(episode.id)
                    is DeviceDownloads.State.Paused,
                    is DeviceDownloads.State.Failed ->
                        models?.device?.download(episode)
                    is DeviceDownloads.State.None -> when {
                        serverRunning -> models?.serverDownloads?.watch(episode.id)
                        onServer -> {
                            if (core.isEmbeddedAccount) {
                                // Optimistic overlay first — the trash flips
                                // back to a download glyph immediately.
                                models?.serverDownloads?.markRemovedLocally(episode.id)
                                core.scope.launch {
                                    core.outbox?.enqueue(
                                        OutboxOp.Kind.RemoveServerDownload(episode.id))
                                }
                            } else {
                                models?.device?.download(episode)
                            }
                        }
                        core.isEmbeddedAccount ->
                            models?.serverDownloads?.download(episode)
                        // Full chain: trigger the server fetch, wait it out
                        // (cloud ring), then pull to the device. Server-only
                        // downloads live in the context menu.
                        else -> models?.device?.download(episode)
                    }
                }
            },
        contentAlignment = Alignment.Center,
    ) {
        when (device) {
            is DeviceDownloads.State.Downloaded -> DownloadGlyph(
                "trash", MaterialTheme.colorScheme.error, "Remove from device")
            // Server phase of a chained device download — cloud ring tracking
            // the server's own progress.
            is DeviceDownloads.State.WaitingServer -> ProgressRing(
                progress = serverProgress ?: 0.0,
                glyph = "icloud.and.arrow.down",
                tint = MaterialTheme.colorScheme.primary,
            )
            is DeviceDownloads.State.Downloading -> ProgressRing(
                progress = device.progress,
                glyph = "arrow.down",
                tint = MaterialTheme.colorScheme.primary,
            )
            is DeviceDownloads.State.Paused -> DownloadGlyph(
                "pause.circle", halogenExtras.warning, "Download paused — tap to resume")
            // Surfaced failure (tap retries) — never disguised as a pause
            // (web: the Failed badge).
            is DeviceDownloads.State.Failed -> DownloadGlyph(
                "exclamationmark.circle",
                MaterialTheme.colorScheme.error,
                "Download failed: ${device.message} Tap to retry.",
            )
            is DeviceDownloads.State.None -> when {
                serverRunning -> ProgressRing(
                    progress = serverProgress ?: 0.0,
                    glyph = "icloud.and.arrow.down",
                    tint = MaterialTheme.colorScheme.primary,
                )
                // Failed server fetch — distinct from "never downloaded";
                // tap retries the chain.
                serverFailed && !onServer -> DownloadGlyph(
                    "exclamationmark.icloud",
                    MaterialTheme.colorScheme.error,
                    "Server download failed — tap to retry",
                )
                onServer && core.isEmbeddedAccount -> DownloadGlyph(
                    "trash", MaterialTheme.colorScheme.error, "Remove download")
                onServer -> DownloadGlyph(
                    "icloud.and.arrow.down",
                    MaterialTheme.colorScheme.primary,
                    "Download to device",
                )
                else -> DownloadGlyph(
                    "arrow.down.circle", MaterialTheme.colorScheme.primary, "Download")
            }
        }
    }
}

@Composable
private fun DownloadGlyph(sf: String, tint: Color, contentDescription: String) {
    Icon(
        halogenIcon(sf),
        contentDescription = contentDescription,
        tint = tint,
        modifier = Modifier.size(22.dp),
    )
}

/// Progress ring wrapped around a glyph — the shared spinner treatment.
@Composable
private fun ProgressRing(progress: Double, glyph: String, tint: Color) {
    val track = MaterialTheme.colorScheme.outlineVariant
    Box(Modifier.size(20.dp), contentAlignment = Alignment.Center) {
        Canvas(Modifier.fillMaxSize()) {
            val strokeWidth = 2.dp.toPx()
            val stroke = Stroke(width = strokeWidth, cap = StrokeCap.Round)
            // Inset so the stroke stays inside the 20dp box.
            val inset = strokeWidth / 2
            val arcSize = Size(size.width - strokeWidth, size.height - strokeWidth)
            drawCircle(
                color = track,
                radius = (size.minDimension - strokeWidth) / 2,
                style = stroke,
            )
            drawArc(
                color = tint,
                startAngle = -90f,
                sweepAngle = (360.0 * max(progress, 0.04)).toFloat(),
                useCenter = false,
                topLeft = Offset(inset, inset),
                size = arcSize,
                style = stroke,
            )
        }
        Icon(
            halogenIcon(glyph),
            contentDescription = "Downloading",
            tint = tint,
            modifier = Modifier.size(11.dp),
        )
    }
}
