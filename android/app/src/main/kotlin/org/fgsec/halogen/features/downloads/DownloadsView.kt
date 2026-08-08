package org.fgsec.halogen.features.downloads

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
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
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
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.flow.drop
import kotlinx.coroutines.launch
import org.fgsec.halogen.components.ConfiguredSwipes
import org.fgsec.halogen.components.HalogenNavbar
import org.fgsec.halogen.components.EpisodeMenuContext
import org.fgsec.halogen.components.EpisodeRowLink
import org.fgsec.halogen.components.ExtraSwipe
import org.fgsec.halogen.components.ListControlsBar
import org.fgsec.halogen.components.LoadErrorView
import org.fgsec.halogen.components.LoadMoreRow
import org.fgsec.halogen.components.halogenIcon
import org.fgsec.halogen.core.HalogenCore
import org.fgsec.halogen.core.SwipePage
import org.fgsec.halogen.features.podcasts.BulkActionBar
import org.fgsec.halogen.wire.EpisodeData

/// The Downloads tab: On-device / Server / Downloading facets, swipe to
/// delete the local copy or the server file.
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun DownloadsView(model: DownloadsModel, core: HalogenCore) {
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

    LaunchedEffect(model.facet, model.query) { model.load() }
    // On-device facet reactivity: recompute when the device set changes
    // (see LatestView's identical hook).
    LaunchedEffect(Unit) {
        snapshotFlow { core.models?.device?.onDevice?.map { it.id } ?: emptyList() }
            .drop(1)
            .collect { if (model.facet == DownloadsModel.Facet.OnDevice) model.refresh() }
    }
    // Server facets: a download finishing (or failing) while the user
    // watches must move the row between Downloading/Server live.
    LaunchedEffect(Unit) {
        snapshotFlow {
            val downloads = core.models?.serverDownloads
            (downloads?.completed ?: emptySet()).sorted() +
                (downloads?.failed ?: emptySet()).sorted()
        }
            .drop(1)
            .collect { if (model.facet != DownloadsModel.Facet.OnDevice) model.refresh() }
    }
    // Leaving Edit closes multi-select for real (the bulk bar is keyed on a
    // non-empty selection).
    LaunchedEffect(editing) {
        if (!editing) {
            selection = emptySet()
            selectedOnly = false
        }
    }

    Column(Modifier.fillMaxSize()) {
        // Was the only tab root with no navbar: no title, dot or account
        // menu. Edit rides the leading slot like every other list.
        HalogenNavbar(core = core, title = "Downloads", leading = {
            TextButton(onClick = { editing = !editing }) {
                Text(if (editing) "Done" else "Edit")
            }
        })
        // Its own row: sharing one with Edit wrapped "Downlo/ading".
        SingleChoiceSegmentedButtonRow(
            Modifier.fillMaxWidth().padding(horizontal = 16.dp).padding(top = 4.dp),
        ) {
            val facets = model.availableFacets
            facets.forEachIndexed { index, facet ->
                SegmentedButton(
                    selected = model.facet == facet,
                    onClick = { model.facet = facet },
                    shape = SegmentedButtonDefaults.itemShape(
                        index = index, count = facets.size),
                ) {
                    Text(facet.label, maxLines = 1, overflow = TextOverflow.Ellipsis)
                }
            }
        }
        // Downloads fixes its facet via the segmented control — no filter menu.
        ListControlsBar(
            query = model.query,
            onQueryChange = { model.query = it },
            showFilter = false,
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
                    title = "Couldn't load downloads", message = error) { model.refresh() }
                model.loaded && model.episodes.isEmpty() ->
                    if (model.query.search.isNotEmpty() || model.query.filters.isNotEmpty()) {
                        DownloadsEmptyState(
                            icon = "line.3.horizontal.decrease.circle",
                            title = "No matches",
                            description = "No downloads match the current search or filters.",
                        )
                    } else {
                        DownloadsEmptyState(
                            icon = "arrow.down.circle",
                            title = "No ${model.facet.label.lowercase()} episodes",
                            description = "Trigger downloads from an episode's context menu.",
                        )
                    }
                else -> LazyColumn(Modifier.fillMaxSize()) {
                    items(displayed, key = { it.id }) { episode ->
                        DownloadsRow(
                            episode = episode,
                            model = model,
                            core = core,
                            editing = editing,
                            selected = episode.id in selection,
                            onSelectedChange = { checked ->
                                selection =
                                    if (checked) selection + episode.id
                                    else selection - episode.id
                            },
                            remove = { scope.launch { model.removeDownload(episode) } },
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

/// One list row: checkbox in edit mode, configured swipes + the destructive
/// remove swipe otherwise.
@Composable
private fun DownloadsRow(
    episode: EpisodeData,
    model: DownloadsModel,
    core: HalogenCore,
    editing: Boolean,
    selected: Boolean,
    onSelectedChange: (Boolean) -> Unit,
    remove: () -> Unit,
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
        ConfiguredSwipes(
            page = SwipePage.Downloads,
            episode = episode,
            core = core,
            extraTrailing = ExtraSwipe(
                label = if (model.facet == DownloadsModel.Facet.OnDevice) "Remove"
                else "Delete file",
                systemImage = "trash",
                destructive = true,
                onClick = remove,
            ),
        ) {
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
private fun DownloadsEmptyState(icon: String, title: String, description: String) {
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
