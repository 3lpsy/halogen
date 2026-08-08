package org.fgsec.halogen.features.history

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.Checkbox
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
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
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch
import org.fgsec.halogen.components.ConfiguredSwipes
import org.fgsec.halogen.components.HalogenNavbar
import org.fgsec.halogen.components.EpisodeMenuContext
import org.fgsec.halogen.components.EpisodeRowLink
import org.fgsec.halogen.components.ListControlsBar
import org.fgsec.halogen.components.LoadErrorView
import org.fgsec.halogen.components.LoadMoreRow
import org.fgsec.halogen.components.halogenIcon
import org.fgsec.halogen.core.HalogenCore
import org.fgsec.halogen.core.SwipePage
import org.fgsec.halogen.features.podcasts.BulkActionBar
import org.fgsec.halogen.wire.EpisodeData

/// History: recently played episodes (in-progress + finished), most recent
/// playback first — the web sorts its local pool by playback recency; here
/// the paged playback rows are merged into a pool, cached for offline.
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun HistoryView(model: HistoryModel, core: HalogenCore) {
    val scope = rememberCoroutineScope()
    var selection by remember { mutableStateOf(setOf<Int>()) }
    var selectedOnly by remember { mutableStateOf(false) }
    // Row selection is an EDIT-MODE tool only (iOS parity): rows select via
    // checkboxes while editing, and leaving Edit clears the set (closes the
    // bulk bar).
    var editing by remember { mutableStateOf(false) }
    var refreshing by remember { mutableStateOf(false) }

    // The "Selected" review chip restricts rows to the ticked set.
    val displayed =
        if (selectedOnly) model.episodes.filter { it.id in selection } else model.episodes

    LaunchedEffect(model.query) { model.load() }
    // Leaving Edit closes multi-select for real (the bulk bar is keyed on a
    // non-empty selection).
    LaunchedEffect(editing) {
        if (!editing) {
            selection = emptySet()
            selectedOnly = false
        }
    }

    Column(Modifier.fillMaxSize()) {
        // Was a bare Edit row with no navbar (iOS keeps title/dot/menu).
        HalogenNavbar(core = core, title = "History", leading = {
            TextButton(onClick = { editing = !editing }) {
                Text(if (editing) "Done" else "Edit")
            }
        })
        // Full filter set on History (web parity) — the chips AND with the
        // intrinsic played/finished membership.
        ListControlsBar(
            query = model.query,
            onQueryChange = { model.query = it },
            allowsRecency = true,
            allowsOnDevice = !core.isEmbeddedAccount,
        )
        PullToRefreshBox(
            isRefreshing = refreshing,
            onRefresh = {
                scope.launch {
                    refreshing = true
                    model.refresh()
                    refreshing = false
                }
            },
            modifier = Modifier.weight(1f).fillMaxWidth(),
        ) {
            val error = model.error
            when {
                error != null -> LoadErrorView(
                    title = "Couldn't load history", message = error) { model.refresh() }
                model.loaded && model.episodes.isEmpty() ->
                    // "Nothing played yet" over a filtered/searched view read
                    // as a wiped history — distinguish no-match from truly empty.
                    if (model.query.search.isNotEmpty() || model.query.filters.isNotEmpty()) {
                        HistoryEmptyState(
                            icon = "line.3.horizontal.decrease.circle",
                            title = "No matches",
                            description = "No played episodes match the current search or filters.",
                        )
                    } else {
                        HistoryEmptyState(
                            icon = "clock.arrow.circlepath",
                            title = "Nothing played yet",
                            description = "Episodes you play appear here.",
                        )
                    }
                else -> LazyColumn(Modifier.fillMaxSize()) {
                    items(displayed, key = { it.id }) { episode ->
                        HistoryRow(
                            episode = episode,
                            core = core,
                            editing = editing,
                            selected = episode.id in selection,
                            onSelectedChange = { checked ->
                                selection =
                                    if (checked) selection + episode.id
                                    else selection - episode.id
                            },
                        )
                    }
                    if (model.hasMore && model.episodes.isNotEmpty() && !selectedOnly) {
                        item { LoadMoreRow(failed = model.loadMoreFailed) { model.loadMore() } }
                    }
                }
            }
        }
        if (selection.isNotEmpty()) {
            BulkActionBar(
                selection = selection,
                onSelectionChange = { selection = it },
                selectedOnly = selectedOnly,
                onSelectedOnlyChange = { selectedOnly = it },
                episodes = model.episodes,
                core = core,
            )
        }
    }
}

/// One list row: checkbox in edit mode, configured swipes otherwise.
@Composable
private fun HistoryRow(
    episode: EpisodeData,
    core: HalogenCore,
    editing: Boolean,
    selected: Boolean,
    onSelectedChange: (Boolean) -> Unit,
) {
    if (editing) {
        Row(
            Modifier.fillMaxWidth().padding(start = 4.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Checkbox(checked = selected, onCheckedChange = onSelectedChange)
            Box(Modifier.weight(1f)) {
                EpisodeRowLink(
                    episode = episode,
                    artUrl = core.episodeArtUrl(episode),
                    subtitle = episode.podcast?.title,
                    context = EpisodeMenuContext.Browse,
                    core = core,
                )
            }
        }
    } else {
        ConfiguredSwipes(page = SwipePage.History, episode = episode, core = core) {
            EpisodeRowLink(
                episode = episode,
                artUrl = core.episodeArtUrl(episode),
                subtitle = episode.podcast?.title,
                context = EpisodeMenuContext.Browse,
                core = core,
            )
        }
    }
}

/// iOS ContentUnavailableView parity.
@Composable
private fun HistoryEmptyState(icon: String, title: String, description: String) {
    Column(
        Modifier.fillMaxSize().padding(32.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp, Alignment.CenterVertically),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Icon(
            halogenIcon(icon),
            contentDescription = null,
            tint = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.size(44.dp),
        )
        Text(title, style = MaterialTheme.typography.titleMedium)
        Text(
            description,
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center,
        )
    }
}
