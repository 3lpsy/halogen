@file:OptIn(ExperimentalMaterial3Api::class)

package org.fgsec.halogen.features.player

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Slider
import androidx.compose.material3.SliderDefaults
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import kotlin.math.max
import org.fgsec.halogen.components.Artwork
import org.fgsec.halogen.components.halogenIcon
import org.fgsec.halogen.core.ClientPrefs
import org.fgsec.halogen.core.HalogenCore
import org.fgsec.halogen.wire.EpisodeData

/// The full player: up-next preview, floating artwork, title, scrubber with chapter
/// ticks, transport, then chapters/rate/sleep/auto-advance anchored at the bottom —
/// the web's NowPlayingScreen surface. Mirrors ios Features/Player/PlayerSheet.swift.
@Composable
fun PlayerSheet(player: PlayerModel, core: HalogenCore, onDismiss: () -> Unit) {
    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
    ) {
        val episode = player.current
        if (episode == null) {
            Spacer(Modifier.height(200.dp))
            return@ModalBottomSheet
        }
        // Everything below the art is fixed-height; the art takes what is
        // left. A fixed 260dp pushed the transport off the bottom at larger
        // UI sizes, and the sheet does not scroll by itself.
        BoxWithConstraints(Modifier.fillMaxSize()) {
            val art = (maxHeight * 0.30f).coerceIn(112.dp, 260.dp)
                .coerceAtMost(maxWidth - 96.dp)
            Column(
                Modifier
                    .fillMaxSize()
                    .verticalScroll(rememberScrollState())
                    .navigationBarsPadding()
                    .padding(bottom = 12.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                player.upNext?.let { next -> UpNextRow(next, core, player) }

                Artwork(url = core.episodeArtUrl(episode, small = false), size = art)

                TitleCluster(player, episode)

                Scrubber(player)

                TransportRow(player)

                ControlsRow(player, core)
            }
        }
    }
}

/// "Up next" preview — the continuation neighbor (queue, or the playlist
/// played from), the web's top-right UpNext surface. Tap skips to it.
@Composable
private fun UpNextRow(next: EpisodeData, core: HalogenCore, player: PlayerModel) {
    Surface(
        shape = RoundedCornerShape(12.dp),
        color = MaterialTheme.colorScheme.surfaceContainerHigh,
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 24.dp)
            .testTag("player-up-next"),
        onClick = { player.playNextEpisode() },
    ) {
        Row(
            Modifier.padding(10.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            Artwork(url = core.episodeArtUrl(next), size = 36.dp)
            Column(Modifier.weight(1f)) {
                Text(
                    "UP NEXT",
                    style = MaterialTheme.typography.labelSmall,
                    fontWeight = FontWeight.SemiBold,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Text(
                    next.title,
                    style = MaterialTheme.typography.bodySmall,
                    fontWeight = FontWeight.Medium,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                next.podcast?.title?.let { podcast ->
                    Text(
                        podcast,
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
            }
            Icon(
                halogenIcon("forward.end.fill"),
                contentDescription = null,
                tint = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.size(16.dp),
            )
        }
    }
}

@Composable
private fun TitleCluster(player: PlayerModel, episode: EpisodeData) {
    Column(
        Modifier.padding(horizontal = 24.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        Text(
            episode.title,
            style = MaterialTheme.typography.titleMedium,
            textAlign = TextAlign.Center,
            maxLines = 2,
            overflow = TextOverflow.Ellipsis,
            modifier = Modifier.testTag("player-title"),
        )
        episode.podcast?.title?.let { podcast ->
            Text(
                podcast,
                style = MaterialTheme.typography.labelMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        val failure = player.failureMessage
        when {
            failure != null ->
                // Tappable — retry routes through the full source rebuild
                // (toggle() handles the failed-item case).
                Row(
                    Modifier
                        .clickable { player.toggle() }
                        .testTag("player-retry"),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(6.dp),
                ) {
                    Icon(
                        halogenIcon("arrow.clockwise.circle.fill"),
                        contentDescription = null,
                        tint = MaterialTheme.colorScheme.error,
                        modifier = Modifier.size(16.dp),
                    )
                    Text(
                        failure,
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.error,
                        textAlign = TextAlign.Center,
                    )
                }
            player.preparing ->
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(6.dp),
                ) {
                    CircularProgressIndicator(Modifier.size(14.dp), strokeWidth = 2.dp)
                    Text(
                        player.preparingLabel,
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            player.streaming ->
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(4.dp),
                ) {
                    Icon(
                        halogenIcon("dot.radiowaves.left.and.right"),
                        contentDescription = null,
                        tint = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.size(14.dp),
                    )
                    Text(
                        "Streaming",
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
        }
    }
}

@Composable
private fun Scrubber(player: PlayerModel) {
    var scrubbing by remember { mutableStateOf(false) }
    var scrubValue by remember { mutableFloatStateOf(0f) }
    val shown = if (scrubbing) scrubValue.toDouble() else player.position
    val tickColor = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.6f)

    Column(
        Modifier
            .fillMaxWidth()
            .padding(horizontal = 24.dp),
        verticalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        // Current chapter — updates as playback crosses each marker
        // (web: the title above the seek bar).
        player.activeChapterIndex?.let { idx ->
            Text(
                "${idx + 1}. ${player.chapters[idx].title}",
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                modifier = Modifier.fillMaxWidth(),
            )
        }
        val duration = max(player.duration, 1.0)
        val chapters = player.chapters
        Slider(
            value = shown.toFloat().coerceIn(0f, duration.toFloat()),
            onValueChange = {
                scrubbing = true
                scrubValue = it
            },
            onValueChangeFinished = {
                scrubbing = false
                player.seek(scrubValue.toDouble())
            },
            valueRange = 0f..duration.toFloat(),
            modifier = Modifier
                .fillMaxWidth()
                // Chapter tick marks — decorative, the web's absolute-
                // positioned lines over the range input.
                .drawWithContent {
                    drawContent()
                    if (player.duration > 0 && chapters.size > 1) {
                        for (chapter in chapters) {
                            val frac = chapter.starts_at_secs / player.duration
                            if (frac > 0 && frac < 1) {
                                val x = size.width * frac.toFloat()
                                drawLine(
                                    color = tickColor,
                                    start = Offset(x, size.height / 2 - 4.dp.toPx()),
                                    end = Offset(x, size.height / 2 + 4.dp.toPx()),
                                    strokeWidth = 1.dp.toPx(),
                                )
                            }
                        }
                    }
                },
            track = { state ->
                SliderDefaults.Track(sliderState = state, drawStopIndicator = null)
            },
        )
        Row(Modifier.fillMaxWidth()) {
            Text(
                formatTime(shown),
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Spacer(Modifier.weight(1f))
            Text(
                "-" + formatTime(max(0.0, player.duration - shown)),
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

@Composable
private fun TransportRow(player: PlayerModel) {
    Row(
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        IconButton(onClick = { player.playPreviousEpisode() }, enabled = player.hasPrevious) {
            Icon(
                halogenIcon("backward.end.fill"),
                contentDescription = "Previous episode",
                modifier = Modifier.size(28.dp),
            )
        }
        IconButton(onClick = { player.skip(-player.skipBackSecs) }) {
            Icon(
                halogenIcon("gobackward"),
                contentDescription = "Skip back",
                modifier = Modifier.size(32.dp),
            )
        }
        // Filled circle, as on iOS — a bare glyph read as one more icon in
        // the row rather than the primary control.
        Surface(
            shape = CircleShape,
            color = MaterialTheme.colorScheme.onSurface,
            onClick = { player.toggle() },
            modifier = Modifier.size(64.dp),
        ) {
            Box(contentAlignment = Alignment.Center) {
                if (player.buffering) {
                    CircularProgressIndicator(
                        Modifier.size(32.dp),
                        color = MaterialTheme.colorScheme.surface,
                    )
                } else {
                    Icon(
                        halogenIcon(if (player.isPlaying) "pause.fill" else "play.fill"),
                        contentDescription = if (player.isPlaying) "Pause" else "Play",
                        tint = MaterialTheme.colorScheme.surface,
                        modifier = Modifier.size(34.dp),
                    )
                }
            }
        }
        IconButton(onClick = { player.skip(player.skipForwardSecs) }) {
            Icon(
                halogenIcon("goforward"),
                contentDescription = "Skip forward",
                modifier = Modifier.size(32.dp),
            )
        }
        IconButton(
            onClick = { player.playNextEpisode() },
            enabled = player.hasNext,
            modifier = Modifier.testTag("player-next"),
        ) {
            Icon(
                halogenIcon("forward.end.fill"),
                contentDescription = "Next episode",
                modifier = Modifier.size(28.dp),
            )
        }
    }
}

@Composable
private fun ControlsRow(player: PlayerModel, core: HalogenCore) {
    Row(
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        if (player.chapters.isNotEmpty()) {
            ChaptersMenu(player)
        }

        RateMenu(player)

        // Auto-advance quick toggle — same pref as Settings (web: the
        // now-playing "A" toggle persists immediately).
        val autoAdvance = core.models?.prefs?.prefs?.autoAdvance ?: true
        IconButton(
            onClick = { core.models?.prefs?.update { it.copy(autoAdvance = !it.autoAdvance) } }
        ) {
            Box(
                Modifier
                    .size(24.dp)
                    .then(
                        if (autoAdvance)
                            Modifier.background(MaterialTheme.colorScheme.primary, CircleShape)
                        else
                            Modifier.border(
                                1.5.dp, MaterialTheme.colorScheme.onSurfaceVariant, CircleShape
                            )
                    ),
                contentAlignment = Alignment.Center,
            ) {
                Text(
                    "A",
                    style = MaterialTheme.typography.labelSmall,
                    fontWeight = FontWeight.Bold,
                    color =
                        if (autoAdvance) MaterialTheme.colorScheme.onPrimary
                        else MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }

        SleepMenu(player, core)
    }
}

/// Chapter picker — selecting a marker seeks to it (web: the drop-up
/// chapters menu). The active marker is checked.
@Composable
private fun ChaptersMenu(player: PlayerModel) {
    var expanded by remember { mutableStateOf(false) }
    Box {
        IconButton(onClick = { expanded = true }) {
            Icon(halogenIcon("list.bullet"), contentDescription = "Chapters")
        }
        DropdownMenu(expanded = expanded, onDismissRequest = { expanded = false }) {
            player.chapters.forEachIndexed { index, chapter ->
                DropdownMenuItem(
                    text = {
                        Text("${formatTime(chapter.starts_at_secs.toDouble())}  ${chapter.title}")
                    },
                    leadingIcon =
                        if (player.activeChapterIndex == index) {
                            {
                                Icon(halogenIcon("checkmark"), contentDescription = null)
                            }
                        } else null,
                    onClick = {
                        expanded = false
                        player.seek(chapter.starts_at_secs.toDouble())
                    },
                )
            }
        }
    }
}

@Composable
private fun RateMenu(player: PlayerModel) {
    var expanded by remember { mutableStateOf(false) }
    Box {
        Chip(label = rateLabel(player.rate), onClick = { expanded = true })
        DropdownMenu(expanded = expanded, onDismissRequest = { expanded = false }) {
            for (rate in ClientPrefs.playbackRates) {
                DropdownMenuItem(
                    text = { Text(rateLabel(rate)) },
                    onClick = {
                        expanded = false
                        player.applyRate(rate)
                    },
                )
            }
        }
    }
}

@Composable
private fun SleepMenu(player: PlayerModel, core: HalogenCore) {
    var expanded by remember { mutableStateOf(false) }
    // The quick sleep durations, always including the configured default
    // (Settings → Playback) so it's armable from here.
    val preset = core.models?.prefs?.prefs?.defaultSleepMinutes ?: 30
    val minutes = (listOf(5, 15, 30, 60) + preset).distinct().sorted()
    Box {
        Chip(label = sleepLabel(player), icon = "moon.zzz", onClick = { expanded = true })
        DropdownMenu(expanded = expanded, onDismissRequest = { expanded = false }) {
            DropdownMenuItem(
                text = { Text("Off") },
                onClick = {
                    expanded = false
                    player.setSleepTimer(null)
                },
            )
            for (m in minutes) {
                DropdownMenuItem(
                    text = { Text("$m min") },
                    onClick = {
                        expanded = false
                        player.setSleepTimer(m)
                    },
                )
            }
            DropdownMenuItem(
                text = { Text("End of episode") },
                onClick = {
                    expanded = false
                    player.setSleepTimer(0)
                },
            )
        }
    }
}

@Composable
private fun Chip(label: String, icon: String? = null, onClick: () -> Unit) {
    Surface(
        shape = CircleShape,
        color = MaterialTheme.colorScheme.surfaceContainerHigh,
        onClick = onClick,
    ) {
        Row(
            Modifier.padding(horizontal = 14.dp, vertical = 6.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            if (icon != null) {
                Icon(
                    halogenIcon(icon),
                    contentDescription = null,
                    modifier = Modifier.size(16.dp),
                )
            }
            Text(
                label,
                style = MaterialTheme.typography.labelMedium,
                fontWeight = FontWeight.SemiBold,
            )
        }
    }
}

private fun sleepLabel(player: PlayerModel): String {
    if (player.sleepAtEpisodeEnd) return "Sleep: end"
    player.sleepRemainingMinutes?.let { return "Sleep: ${it}m" }
    return "Sleep"
}

/// Swift's %g rate formatting: "1×", "1.25×" — no trailing zeros.
private fun rateLabel(rate: Float): String {
    val text =
        if (rate == rate.toInt().toFloat()) rate.toInt().toString()
        else rate.toString().trimEnd('0').trimEnd('.')
    return "$text×"
}

private fun formatTime(seconds: Double): String {
    val s = seconds.toInt().coerceAtLeast(0)
    return if (s >= 3600) "%d:%02d:%02d".format(s / 3600, (s % 3600) / 60, s % 60)
    else "%d:%02d".format(s / 60, s % 60)
}
