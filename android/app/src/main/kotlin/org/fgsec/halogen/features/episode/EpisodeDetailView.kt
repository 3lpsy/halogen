package org.fgsec.halogen.features.episode

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.CenterAlignedTopAppBar
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.util.Locale
import kotlin.math.min
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.launch
import org.fgsec.halogen.components.AppRoute
import org.fgsec.halogen.components.Artwork
import org.fgsec.halogen.components.DownloadButton
import org.fgsec.halogen.components.EpisodeMenu
import org.fgsec.halogen.components.EpisodeMenuContext
import org.fgsec.halogen.components.EpisodeMenuItem
import org.fgsec.halogen.components.HtmlDescription
import org.fgsec.halogen.components.LoadErrorView
import org.fgsec.halogen.components.LocalNavigator
import org.fgsec.halogen.components.halogenExtras
import org.fgsec.halogen.components.halogenIcon
import org.fgsec.halogen.core.DeviceDownloads
import org.fgsec.halogen.core.HalogenCore
import org.fgsec.halogen.networking.WireJson
import org.fgsec.halogen.wire.DownloadStatus
import org.fgsec.halogen.wire.EpisodeData
import org.fgsec.halogen.wire.PlaybackStatus

/// Episode detail: full metadata, actions (play / queue / download / played),
/// description, chapters. Local-first — the fetched episode caches per id, so
/// anything opened once re-renders offline.
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun EpisodeDetailView(
    core: HalogenCore,
    episodeId: Int,
    onBack: (() -> Unit)? = null,
) {
    val model = remember(core, episodeId) { EpisodeDetailModel(core, episodeId) }
    val navigator = LocalNavigator.current
    val scope = rememberCoroutineScope()
    var refreshing by remember { mutableStateOf(false) }
    var menuOpen by remember { mutableStateOf(false) }

    LaunchedEffect(model) { model.load() }
    // A row already DOWNLOADING server-side gets live tracking (the web's
    // durable-status poll) …
    LaunchedEffect(model.episode?.download_status) {
        if (model.episode?.download_status == DownloadStatus.Downloading) {
            core.models?.serverDownloads?.watch(episodeId)
        }
    }
    // … and the tracked outcome (success OR failure) refreshes the snapshot,
    // so the page never stays "downloading" until a manual refresh.
    val serverOutcome = core.models?.serverDownloads?.let {
        episodeId in it.completed || episodeId in it.failed
    } == true
    LaunchedEffect(serverOutcome) { if (serverOutcome) model.refresh() }
    // An offline load failure heals itself on reconnect (no manual retry).
    LaunchedEffect(core.isOffline) {
        if (!core.isOffline && model.error != null) model.refresh()
    }

    Scaffold(
        topBar = {
            CenterAlignedTopAppBar(
                title = { Text("Episode") },
                navigationIcon = {
                    if (onBack != null) {
                        IconButton(onClick = onBack) {
                            Icon(halogenIcon("chevron.left"), contentDescription = "Back")
                        }
                    }
                },
                actions = {
                    // The web's detail kebab: the SAME shared menu the list
                    // rows use (stream / downloads / playlist / queue /
                    // played / purge), plus View metadata.
                    Box {
                        IconButton(onClick = { menuOpen = true }) {
                            Icon(
                                halogenIcon("ellipsis.circle"),
                                contentDescription = "Episode menu",
                            )
                        }
                        DropdownMenu(
                            expanded = menuOpen,
                            onDismissRequest = { menuOpen = false },
                        ) {
                            model.episode?.let { episode ->
                                EpisodeMenu(
                                    episode = episode,
                                    context = EpisodeMenuContext.Browse,
                                    core = core,
                                    dismiss = { menuOpen = false },
                                )
                                HorizontalDivider()
                            }
                            EpisodeMenuItem("View metadata", "info.circle") {
                                navigator?.push(AppRoute.EpisodeMetadata(episodeId))
                                menuOpen = false
                            }
                        }
                    }
                },
            )
        },
    ) { padding ->
        PullToRefreshBox(
            isRefreshing = refreshing,
            onRefresh = {
                scope.launch {
                    refreshing = true
                    model.refresh()
                    refreshing = false
                }
            },
            modifier = Modifier.padding(padding).fillMaxSize(),
        ) {
            val episode = model.episode
            val error = model.error
            when {
                episode != null -> Content(core = core, model = model, episode = episode)
                error != null -> LoadErrorView(
                    title = "Couldn't load episode",
                    message = error,
                    retry = { model.refresh() },
                )
                else -> Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                    CircularProgressIndicator()
                }
            }
        }
    }
}

@Composable
private fun Content(core: HalogenCore, model: EpisodeDetailModel, episode: EpisodeData) {
    val navigator = LocalNavigator.current
    val scope = rememberCoroutineScope()

    Column(
        Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState())
            .padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        Row(horizontalArrangement = Arrangement.spacedBy(14.dp)) {
            Artwork(url = core.episodeArtUrl(episode, small = false), size = 96.dp)
            Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
                Text(episode.title, style = MaterialTheme.typography.titleMedium)
                val podcast = episode.podcast
                if (podcast != null) {
                    Text(
                        podcast.title,
                        style = MaterialTheme.typography.labelMedium,
                        color = MaterialTheme.colorScheme.primary,
                        modifier = Modifier.clickable {
                            navigator?.push(AppRoute.Podcast(podcast.id))
                        },
                    )
                }
                MetaLine(episode, core)
            }
        }

        Actions(core = core, model = model, episode = episode, scope = scope)

        // Playback-position bar (overlay-wins cursor / duration) — the web
        // detail's progress_pct track.
        val progress = playbackProgress(episode, core)
        if (progress != null) {
            LinearProgressIndicator(
                progress = { progress.toFloat() },
                modifier = Modifier.fillMaxWidth(),
                color = MaterialTheme.colorScheme.primary,
            )
        }

        val description = episode.description
        if (!description.isNullOrEmpty()) {
            HorizontalDivider()
            HtmlDescription(description)
        }

        val chapters = episode.chapters
        if (!chapters.isNullOrEmpty()) {
            HorizontalDivider()
            Text("Chapters", style = MaterialTheme.typography.titleMedium)
            for (chapter in chapters) {
                Row(
                    Modifier
                        .fillMaxWidth()
                        .clickable {
                            // Tap-to-seek while THIS episode is playing.
                            val player = core.models?.player
                            if (player?.current?.id == episode.id) {
                                player.seek(chapter.starts_at_secs.toDouble())
                            }
                        }
                        .padding(vertical = 4.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Text(
                        timestamp(chapter.starts_at_secs),
                        style = MaterialTheme.typography.labelSmall
                            .copy(fontFamily = FontFamily.Monospace),
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.width(56.dp),
                    )
                    Text(chapter.title, style = MaterialTheme.typography.bodyMedium)
                }
            }
        }
    }
}

@Composable
private fun MetaLine(episode: EpisodeData, core: HalogenCore) {
    Row(
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        val published = episode.published_at?.let { formatPublished(it) }
        if (published != null) {
            Text(
                published,
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        val secs = episode.duration_secs
        if (secs != null && secs > 0) {
            Text(
                "·",
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Text(
                "${secs / 60} min",
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        if (playedStatus(episode, core) == PlaybackStatus.Finished) {
            Icon(
                halogenIcon("checkmark.circle.fill"),
                contentDescription = "Played",
                tint = halogenExtras.success,
                modifier = Modifier.size(14.dp),
            )
        }
    }
}

@Composable
private fun Actions(
    core: HalogenCore,
    model: EpisodeDetailModel,
    episode: EpisodeData,
    scope: CoroutineScope,
) {
    Row(
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        PrimaryButton(episode = episode, core = core, modifier = Modifier.weight(1f))

        val queue = core.models?.queue
        if (queue != null) {
            val inQueue = queue.contains(episode)
            OutlinedButton(
                onClick = { if (inQueue) queue.remove(episode) else queue.add(episode) },
                contentPadding = PaddingValues(horizontal = 12.dp),
            ) {
                Icon(
                    halogenIcon(if (inQueue) "text.badge.minus" else "text.badge.plus"),
                    contentDescription =
                        if (inQueue) "Remove from Queue" else "Add to Queue",
                    modifier = Modifier.size(20.dp),
                )
            }
        }

        OutlinedButton(
            onClick = { scope.launch { model.togglePlayed() } },
            contentPadding = PaddingValues(horizontal = 12.dp),
        ) {
            val finished = playedStatus(episode, core) == PlaybackStatus.Finished
            Icon(
                halogenIcon(if (finished) "checkmark.circle.fill" else "checkmark.circle"),
                contentDescription = if (finished) "Mark unplayed" else "Mark played",
                modifier = Modifier.size(20.dp),
            )
        }

        // The tiered download control the list rows use (device trash /
        // device+server progress rings / cloud pull / plain download) — one
        // shared component so the tiers can't drift.
        Box(Modifier.padding(horizontal = 4.dp)) {
            DownloadButton(episode = episode, core = core)
        }
    }
}

/// The web detail's primary button: "Download & Play" when the episode is
/// nowhere (plain "Download" on embedded), otherwise Play/Pause with
/// in-button download progress while a copy is being fetched.
@Composable
private fun PrimaryButton(episode: EpisodeData, core: HalogenCore, modifier: Modifier) {
    val models = core.models
    val device: DeviceDownloads.State =
        if (core.isEmbeddedAccount) DeviceDownloads.State.None
        else models?.device?.stateOf(episode.id) ?: DeviceDownloads.State.None
    val onServer = models?.serverDownloads?.isDownloaded(episode)
        ?: (episode.download_status == DownloadStatus.Downloaded)
    val serverProgress = models?.serverDownloads?.progress(episode.id)
    val serverRunning = serverProgress != null ||
        (episode.download_status == DownloadStatus.Downloading &&
            models?.serverDownloads?.completed?.contains(episode.id) != true &&
            models?.serverDownloads?.failed?.contains(episode.id) != true)
    val deviceProgress = (device as? DeviceDownloads.State.Downloading)?.progress
    val isCurrent = models?.player?.current?.id == episode.id
    val preparing = isCurrent && models?.player?.preparing == true
    val nowhere = !onServer && device != DeviceDownloads.State.Downloaded &&
        !serverRunning && deviceProgress == null && !preparing

    if (nowhere) {
        if (core.isEmbeddedAccount) {
            // Embedded: a plain server download — Play then streams from the
            // built-in server (web: the embedded Download branch).
            Button(
                onClick = { models?.serverDownloads?.download(episode) },
                enabled = !core.isOffline,
                modifier = modifier,
            ) {
                Icon(
                    halogenIcon("arrow.down.circle"),
                    contentDescription = null,
                    modifier = Modifier.size(18.dp),
                )
                Spacer(Modifier.width(6.dp))
                Text("Download")
            }
        } else {
            // Force fetch→play with progress (the player's preparation
            // pipeline mirrors the strategy, captions included). Web
            // episode_detail: download_and_play_in(id, None) — playing from
            // the detail resets to queue semantics.
            Button(
                onClick = { models?.player?.play(episode, null) },
                enabled = !core.isOffline,
                modifier = modifier,
            ) {
                Icon(
                    halogenIcon("icloud.and.arrow.down"),
                    contentDescription = null,
                    modifier = Modifier.size(18.dp),
                )
                Spacer(Modifier.width(6.dp))
                Text("Download & Play")
            }
        }
    } else {
        Button(
            onClick = {
                if (isCurrent) {
                    models?.player?.toggle()
                } else {
                    // Web episode_detail: request_play_in(id, None).
                    models?.player?.play(episode, null)
                }
            },
            enabled = !preparing,
            modifier = modifier,
        ) {
            when {
                preparing -> {
                    ButtonSpinner()
                    Spacer(Modifier.width(6.dp))
                    Text("Preparing…")
                }
                deviceProgress != null -> {
                    ButtonSpinner()
                    Spacer(Modifier.width(6.dp))
                    Text("To device… ${(deviceProgress * 100).toInt()}%")
                }
                serverRunning -> {
                    ButtonSpinner()
                    Spacer(Modifier.width(6.dp))
                    Text(
                        serverPercent(
                            serverProgress,
                            if (core.isEmbeddedAccount) "Downloading…" else "On server…",
                        )
                    )
                }
                isCurrent && models?.player?.isPlaying == true -> {
                    Icon(
                        halogenIcon("pause.fill"),
                        contentDescription = null,
                        modifier = Modifier.size(18.dp),
                    )
                    Spacer(Modifier.width(6.dp))
                    Text("Pause")
                }
                else -> {
                    Icon(
                        halogenIcon("play.fill"),
                        contentDescription = null,
                        modifier = Modifier.size(18.dp),
                    )
                    Spacer(Modifier.width(6.dp))
                    Text("Play")
                }
            }
        }
    }
}

@Composable
private fun ButtonSpinner() {
    CircularProgressIndicator(
        modifier = Modifier.size(16.dp),
        color = LocalContentColor.current,
        strokeWidth = 2.dp,
    )
}

private fun serverPercent(progress: Double?, prefix: String): String {
    if (progress == null) return prefix
    return "$prefix ${(progress * 100).toInt()}%"
}

/// Overlay-wins played facet (an offline toggle flips this immediately).
private fun playedStatus(episode: EpisodeData, core: HalogenCore): PlaybackStatus =
    core.models?.playbacks?.status(episode)
        ?: episode.playback_status ?: PlaybackStatus.Unplayed

/// Overlay-wins position fraction for the progress bar.
private fun playbackProgress(episode: EpisodeData, core: HalogenCore): Double? {
    val duration = episode.duration_secs ?: return null
    if (duration <= 0) return null
    val cursor = core.models?.playbacks?.cursor(episode) ?: episode.playback?.cursor
    if (cursor == null || cursor == 0uL) return null
    return min(cursor.toLong().toDouble() / duration.toDouble(), 1.0)
}

private val publishedFormat: DateTimeFormatter =
    DateTimeFormatter.ofPattern("MMM d, yyyy", Locale.US)

private fun formatPublished(raw: String): String? = runCatching {
    publishedFormat.format(WireJson.parseInstant(raw).atZone(ZoneId.systemDefault()))
}.getOrNull()

private fun timestamp(secs: Int): String =
    if (secs >= 3600) {
        "%d:%02d:%02d".format(secs / 3600, (secs % 3600) / 60, secs % 60)
    } else {
        "%d:%02d".format(secs / 60, secs % 60)
    }
