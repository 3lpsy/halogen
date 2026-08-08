package org.fgsec.halogen.features.player

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import org.fgsec.halogen.components.Artwork
import org.fgsec.halogen.components.halogenIcon
import org.fgsec.halogen.core.DeviceDownloads
import org.fgsec.halogen.core.HalogenCore
import org.fgsec.halogen.wire.EpisodeData

/// The persistent mini player above the dock (visible on every tab while
/// something is loaded). Tap opens the full player sheet. Mirrors ios
/// Features/Player/MiniPlayerBar.swift.
@Composable
fun MiniPlayerBar(player: PlayerModel, core: HalogenCore) {
    var showSheet by remember { mutableStateOf(false) }
    val episode = player.current
    if (episode != null) {
        Bar(player = player, core = core, episode = episode, onOpen = { showSheet = true })
    }
    // Terminal stops close the sheet: the condition drops it when current
    // goes null (the iOS onChange-driven dismissal).
    if (showSheet && player.current != null) {
        PlayerSheet(player = player, core = core, onDismiss = { showSheet = false })
    }
}

private fun deviceProgress(core: HalogenCore, episodeId: Int): Double? =
    (core.models?.device?.stateOf(episodeId) as? DeviceDownloads.State.Downloading)?.progress

@Composable
private fun Bar(
    player: PlayerModel,
    core: HalogenCore,
    episode: EpisodeData,
    onOpen: () -> Unit,
) {
    Column(
        Modifier
            .fillMaxWidth()
            .background(MaterialTheme.colorScheme.surfaceContainer)
            .clickable(onClick = onOpen)
    ) {
        HorizontalDivider()
        Row(
            Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp, vertical = 8.dp),
            verticalAlignment = Alignment.CenterVertically,
            // 6dp, not 12: wider gaps starved the title column.
            horizontalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            Artwork(url = core.episodeArtUrl(episode), size = 36.dp)
            Column(Modifier.weight(1f)) {
                Text(
                    episode.title,
                    style = MaterialTheme.typography.bodySmall,
                    fontWeight = FontWeight.Medium,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.testTag("mini-title"),
                )
                val failure = player.failureMessage
                when {
                    failure != null -> Text(
                        failure,
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.error,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                    // Clamped like its siblings: unclamped it wrapped one
                    // character per line and quadrupled the bar's height.
                    player.preparing -> Text(
                        player.preparingLabel,
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                    player.streaming -> Row(
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(4.dp),
                    ) {
                        Icon(
                            halogenIcon("dot.radiowaves.left.and.right"),
                            contentDescription = null,
                            tint = MaterialTheme.colorScheme.onSurfaceVariant,
                            modifier = Modifier.size(12.dp),
                        )
                        Text(
                            "Streaming",
                            style = MaterialTheme.typography.labelSmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                        episode.podcast?.title?.let { podcast ->
                            Text(
                                "· $podcast",
                                style = MaterialTheme.typography.labelSmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis,
                            )
                        }
                    }
                    else -> episode.podcast?.title?.let { podcast ->
                        Text(
                            podcast,
                            style = MaterialTheme.typography.labelSmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                    }
                }
            }
            if (player.preparing) {
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(6.dp),
                ) {
                    CircularProgressIndicator(Modifier.size(16.dp), strokeWidth = 2.dp)
                    val progress = core.models?.serverDownloads?.progress(episode.id)
                        ?: deviceProgress(core, episode.id)
                    if (progress != null) {
                        Text(
                            "${(progress * 100).toInt()}%",
                            style = MaterialTheme.typography.labelSmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
            } else if (player.failureMessage != null) {
                Icon(
                    halogenIcon("exclamationmark.triangle.fill"),
                    contentDescription = player.failureMessage,
                    tint = MaterialTheme.colorScheme.error,
                )
            }
            IconButton(
                onClick = {
                    val current = player.current
                    if (player.failureMessage != null && current != null) {
                        player.play(current)
                    } else {
                        player.toggle()
                    }
                },
                modifier = Modifier.size(36.dp).testTag("mini-play"),
            ) {
                if (player.buffering) {
                    CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp)
                } else {
                    Icon(
                        halogenIcon(if (player.isPlaying) "pause.fill" else "play.fill"),
                        contentDescription = if (player.isPlaying) "Pause" else "Play",
                    )
                }
            }
            // Hidden while preparing: skipping is a no-op then, and the slot
            // is better spent on the (already narrow) title column.
            if (!player.preparing) {
                IconButton(
                    onClick = { player.skip(player.skipForwardSecs) },
                    modifier = Modifier.size(36.dp),
                ) {
                    Icon(halogenIcon("goforward"), contentDescription = "Skip forward")
                }
            }
            // Close/stop — without it the bar is undismissable for the whole
            // session (web's mini player has stop).
            IconButton(
                onClick = { player.stop() },
                modifier = Modifier.size(28.dp).testTag("mini-stop"),
            ) {
                Icon(
                    halogenIcon("xmark"),
                    contentDescription = "Stop",
                    tint = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
        // Thin position track along the bar's bottom edge.
        if (player.duration > 0) {
            Box(Modifier.fillMaxWidth().height(2.dp)) {
                Box(
                    Modifier
                        .fillMaxWidth(
                            fraction = (player.position / player.duration)
                                .coerceIn(0.0, 1.0).toFloat()
                        )
                        .fillMaxHeight()
                        .background(MaterialTheme.colorScheme.primary)
                )
            }
        }
    }
}
