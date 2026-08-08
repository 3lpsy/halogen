package org.fgsec.halogen.components

import android.icu.text.RelativeDateTimeFormatter
import android.icu.text.RelativeDateTimeFormatter.AbsoluteUnit
import android.icu.text.RelativeDateTimeFormatter.RelativeUnit
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
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
import kotlin.math.max
import org.fgsec.halogen.core.HalogenCore
import org.fgsec.halogen.networking.WireJson
import org.fgsec.halogen.wire.EpisodeData
import org.fgsec.halogen.wire.PlaybackStatus

/// Pure display vocabulary for an episode list row (iOS EpisodeRowStyle).
object EpisodeRowStyle {
    /// The web's PlaybackMarker: rows show at most ONE status icon, by
    /// precedence — next-up first (it's what plays when the current episode
    /// ends), then finished, then in-progress.
    enum class PlaybackMarker { None, Finished, InProgress, NextUp }

    fun duration(secs: Int): String {
        // Floor at 1 — a 45-second trailer must not read "0 min" (web parity).
        val mins = max(1, secs / 60)
        if (mins >= 60) {
            return "${mins / 60} hr ${mins % 60} min"
        }
        return "$mins min"
    }
}

/// The single status glyph for a row (nothing for `None`).
@Composable
fun PlaybackMarkerIcon(marker: EpisodeRowStyle.PlaybackMarker, modifier: Modifier = Modifier) {
    when (marker) {
        EpisodeRowStyle.PlaybackMarker.None -> {}
        EpisodeRowStyle.PlaybackMarker.Finished -> Icon(
            halogenIcon("checkmark.circle.fill"),
            contentDescription = null,
            tint = halogenExtras.success,
            modifier = modifier.size(16.dp),
        )
        EpisodeRowStyle.PlaybackMarker.InProgress -> Icon(
            halogenIcon("circle.lefthalf.filled"),
            contentDescription = null,
            tint = MaterialTheme.colorScheme.primary,
            modifier = modifier.size(16.dp),
        )
        EpisodeRowStyle.PlaybackMarker.NextUp -> Icon(
            halogenIcon("arrow.forward.circle"),
            contentDescription = null,
            tint = MaterialTheme.colorScheme.primary,
            modifier = modifier.size(16.dp),
        )
    }
}

/// An episode list row (web item.rs): art, title/subtitle, preview + status icon,
/// transport/download/meta line, progress bar when playing. Navigation is DELIBERATELY
/// narrow — only title/artwork/chevron push detail; long-press opens the ellipsis menu.
@OptIn(ExperimentalFoundationApi::class)
@Composable
fun EpisodeRowLink(
    episode: EpisodeData,
    artUrl: String?,
    subtitle: String? = null,
    context: EpisodeMenuContext,
    core: HalogenCore,
) {
    val navigator = LocalNavigator.current
    var menuOpen by remember { mutableStateOf(false) }
    val pushDetail = { navigator?.push(AppRoute.Episode(episode.id)) ?: Unit }
    val interaction = remember { MutableInteractionSource() }

    Column(
        Modifier
            .fillMaxWidth()
            // Gap taps do nothing (iOS parity); long-press opens the menu.
            .combinedClickable(
                interactionSource = interaction,
                indication = null,
                onClick = {},
                onLongClick = { menuOpen = true },
            )
            // Inset the content, not the divider (iOS rows are inset ~16pt;
            // flush-left rows read as a rendering bug).
            .padding(horizontal = 16.dp, vertical = 8.dp),
        verticalArrangement = Arrangement.spacedBy(5.dp),
    ) {
        // Row 1: artwork | title + podcast (stacked) | chevron.
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Box(Modifier.clickable { pushDetail() }) {
                Artwork(url = artUrl, size = 48.dp)
            }
            Column(
                Modifier.weight(1f),
                verticalArrangement = Arrangement.spacedBy(2.dp),
            ) {
                Text(
                    episode.title,
                    style = MaterialTheme.typography.labelMedium
                        .copy(fontWeight = FontWeight.Medium),
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis,
                    modifier = Modifier
                        .fillMaxWidth()
                        .clickable { pushDetail() }
                        .testTag("row-title"),
                )
                if (!subtitle.isNullOrEmpty()) {
                    Text(
                        subtitle,
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
            }
            Box(
                Modifier.size(width = 24.dp, height = 40.dp).clickable { pushDetail() },
                contentAlignment = Alignment.Center,
            ) {
                Icon(
                    halogenIcon("chevron.right"),
                    contentDescription = null,
                    tint = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }

        // Row 2: one-line description, running to the edge unless the
        // (single, precedence-picked) status icon occupies it. Memoized in
        // HtmlText — row bodies re-render on every overlay/player publish.
        val preview = descriptionPreview(episode)
        val marker = marker(episode, context, core)
        if (preview != null || marker != EpisodeRowStyle.PlaybackMarker.None) {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                if (preview != null) {
                    Text(
                        preview,
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
                Spacer(Modifier.weight(1f))
                PlaybackMarkerIcon(marker)
            }
        }

        // Row 3: play, download/trash, duration, published — and the three
        // dots pulled to the far right (under chevron/status).
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(14.dp),
        ) {
            PlayPauseButton(episode = episode, context = context, core = core)
            DownloadButton(episode = episode, core = core)
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(6.dp),
            ) {
                val secs = episode.duration_secs
                if (secs != null && secs > 0) {
                    Text(
                        EpisodeRowStyle.duration(secs),
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                val published = episode.published_at
                if (published != null) {
                    Text(
                        "·",
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    Text(
                        relativeDate(published),
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
            Spacer(Modifier.weight(1f))
            Box {
                IconButton(
                    onClick = { menuOpen = true },
                    modifier = Modifier.size(width = 28.dp, height = 24.dp),
                ) {
                    Icon(
                        halogenIcon("ellipsis"),
                        contentDescription = "Episode menu",
                        tint = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                DropdownMenu(expanded = menuOpen, onDismissRequest = { menuOpen = false }) {
                    EpisodeMenu(
                        episode = episode, context = context, core = core,
                        dismiss = { menuOpen = false },
                    )
                }
            }
        }

        // Row 4 (optional): live progress — the NOW PLAYING episode only.
        val progress = progress(episode, core)
        if (progress != null) {
            LinearProgressIndicator(
                progress = { progress.toFloat() },
                modifier = Modifier.fillMaxWidth().height(3.dp),
                color = MaterialTheme.colorScheme.primary,
            )
        }
    }
    // Separator, as the iOS list has: without it rows ran together.
    HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant.copy(alpha = 0.4f))
}

/// Play flips to pause while THIS episode is the playing one (tap then
/// pauses in place); a paused current episode shows play and resumes.
@Composable
private fun PlayPauseButton(
    episode: EpisodeData,
    context: EpisodeMenuContext,
    core: HalogenCore,
) {
    val player = core.models?.player
    val isCurrent = player?.current?.id == episode.id
    val playing = isCurrent && (player?.isPlaying ?: false)
    IconButton(
        onClick = {
            if (isCurrent) {
                player?.toggle()
            } else {
                player?.play(episode, context.playbackContext)
            }
        },
        modifier = Modifier.size(width = 30.dp, height = 28.dp).testTag("row-play"),
    ) {
        Icon(
            halogenIcon(if (playing) "pause.circle" else "play.circle"),
            contentDescription = if (playing) "Pause" else "Play",
            tint = MaterialTheme.colorScheme.primary,
        )
    }
}

/// One-line plain-text preview of the (HTML) feed description.
private fun descriptionPreview(episode: EpisodeData): String? {
    val html = episode.description
    if (html.isNullOrEmpty()) return null
    val plain = HtmlText.preview(html)
    return plain.ifEmpty { null }
}

/// The row's marker — next-up outranks the played facet in the queue (web
/// item.rs: the PlaybackMarker priority). Overlay-wins like every reader.
private fun marker(
    episode: EpisodeData,
    context: EpisodeMenuContext,
    core: HalogenCore,
): EpisodeRowStyle.PlaybackMarker {
    if (isNextUp(episode, context, core)) return EpisodeRowStyle.PlaybackMarker.NextUp
    val status = core.models?.playbacks?.status(episode)
        ?: episode.playback_status ?: PlaybackStatus.Unplayed
    return when (status) {
        PlaybackStatus.Finished -> EpisodeRowStyle.PlaybackMarker.Finished
        PlaybackStatus.Played -> EpisodeRowStyle.PlaybackMarker.InProgress
        PlaybackStatus.Unplayed -> EpisodeRowStyle.PlaybackMarker.None
    }
}

/// The episode that plays when the current one ends — shown only in the list playback
/// actually continues through (web next_up_in): the active play-context playlist's
/// own page, else the queue. Without the gate the queue page marked rows it won't play.
private fun isNextUp(
    episode: EpisodeData,
    context: EpisodeMenuContext,
    core: HalogenCore,
): Boolean {
    val contextId = core.models?.player?.contextPlaylistId
    val episodes: List<EpisodeData> = when (context) {
        is EpisodeMenuContext.Queue -> {
            if (contextId != null) return false
            core.models?.queue?.episodes ?: emptyList()
        }
        is EpisodeMenuContext.Playlist -> {
            if (contextId != context.model.playlistId) return false
            context.model.episodes
        }
        is EpisodeMenuContext.Browse -> return false
    }
    if (episodes.isEmpty()) return false
    val currentId = core.models?.player?.current?.id
    val idx = if (currentId != null) episodes.indexOfFirst { it.id == currentId } else -1
    val nextId: Int? = if (idx >= 0) episodes.getOrNull(idx + 1)?.id else episodes.first().id
    return nextId == episode.id
}

/// Thin-track fraction: real progress only (started, not at the end) — a
/// finished episode's cursor resets to 0, so no bar (web compute_progress).
/// The now-playing row tracks the LIVE position; other rows read only the
/// saved cursor.
private fun progress(episode: EpisodeData, core: HalogenCore): Double? {
    val duration = episode.duration_secs ?: return null
    if (duration <= 0) return null
    val player = core.models?.player
    val livePosition =
        if (player != null && player.current?.id == episode.id) player.position else 0.0
    val cursor: Double = if (livePosition > 0) {
        livePosition
    } else {
        val saved = core.models?.playbacks?.cursor(episode) ?: episode.playback?.cursor
        saved?.toLong()?.toDouble() ?: return null
    }
    val frac = cursor / duration.toDouble()
    return if (frac > 0 && frac < 1) frac else null
}

/// "yesterday" / "last month" / "2 months ago" — iOS's relative(.named).
/// DateUtils gives up past a week and prints "Jun 19, 2026" where iOS still
/// says "last month", so ICU does it instead: named for one unit, numeric above.
private fun relativeDate(raw: String): String = runCatching {
    val secs = (System.currentTimeMillis() - WireJson.parseInstant(raw).toEpochMilli()) / 1000.0
    val fmt = RelativeDateTimeFormatter.getInstance()
    val past = RelativeDateTimeFormatter.Direction.LAST
    val (count, relative, absolute) = when {
        secs < 60 -> return "now"
        secs < 3600 -> Triple(secs / 60, RelativeUnit.MINUTES, AbsoluteUnit.MINUTE)
        secs < 86_400 -> Triple(secs / 3600, RelativeUnit.HOURS, AbsoluteUnit.HOUR)
        secs < 604_800 -> Triple(secs / 86_400, RelativeUnit.DAYS, AbsoluteUnit.DAY)
        secs < 2_592_000 -> Triple(secs / 604_800, RelativeUnit.WEEKS, AbsoluteUnit.WEEK)
        secs < 31_536_000 -> Triple(secs / 2_592_000, RelativeUnit.MONTHS, AbsoluteUnit.MONTH)
        else -> Triple(secs / 31_536_000, RelativeUnit.YEARS, AbsoluteUnit.YEAR)
    }
    val whole = count.toInt()
    if (whole <= 1) fmt.format(past, absolute) else fmt.format(whole.toDouble(), past, relative)
}.getOrDefault("")
