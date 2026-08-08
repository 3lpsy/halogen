package org.fgsec.halogen.features.latest

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Modifier
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import kotlinx.coroutines.flow.drop
import kotlinx.coroutines.launch
import org.fgsec.halogen.components.EmptyStateView
import org.fgsec.halogen.components.HalogenNavbar
import org.fgsec.halogen.components.ListControlsBar
import org.fgsec.halogen.components.ListQuery
import org.fgsec.halogen.components.LoadErrorView
import org.fgsec.halogen.components.LoadMoreRow
import org.fgsec.halogen.core.HalogenCore
import org.fgsec.halogen.core.SwipePage
import org.fgsec.halogen.features.podcasts.BulkActionBar
import org.fgsec.halogen.features.podcasts.SelectableEpisodeRow

/// The Latest tab: newest episodes across every subscription, faceted by the
/// filter bar, local-first, with infinite scroll, restored scroll position,
/// configurable swipes, quick-action row menus, and bulk multiselect.
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun LatestView(model: LatestModel, core: HalogenCore) {
    val scope = rememberCoroutineScope()
    var selection by remember { mutableStateOf(setOf<Int>()) }
    var selectedOnly by remember { mutableStateOf(false) }
    /// Row selection is an EDIT-MODE tool only: rows keep their normal taps
    /// until Edit is active, and leaving Edit clears the set (closes the
    /// bulk bar).
    var isEditing by remember { mutableStateOf(false) }
    var refreshing by remember { mutableStateOf(false) }
    val listState: LazyListState =
        rememberSaveable(saver = LazyListState.Saver) { LazyListState() }

    // task(id: query): first appearance and every query change reload.
    LaunchedEffect(model.query) { model.load() }

    // OnDevice facet: the list is a one-shot copy of the device set —
    // recompute when membership changes (a download completing or a
    // removal), or the facet shows stale rows until a manual refresh.
    LaunchedEffect(Unit) {
        snapshotFlow { core.models?.device?.onDevice?.map { it.id } ?: emptyList() }
            .drop(1)
            .collect {
                if (model.query.filters.contains(EpisodeFilter.OnDevice)) model.refresh()
            }
    }

    // Backgrounding (and tab disappear) persist the scroll anchor — it
    // survives relaunch, not just tab switches.
    val lifecycleOwner = LocalLifecycleOwner.current
    DisposableEffect(lifecycleOwner) {
        val observer = LifecycleEventObserver { _, event ->
            if (event == Lifecycle.Event.ON_PAUSE) model.persistScrollAnchor()
        }
        lifecycleOwner.lifecycle.addObserver(observer)
        onDispose {
            lifecycleOwner.lifecycle.removeObserver(observer)
            model.persistScrollAnchor()
        }
    }

    /// The "Selected" review chip restricts rows to the ticked set.
    val displayed =
        if (selectedOnly) model.episodes.filter { it.id in selection } else model.episodes

    // One-shot scroll restore: consume the model's pending target.
    val pending = model.pendingScrollTo
    LaunchedEffect(pending) {
        if (pending != null) {
            val idx = displayed.indexOfFirst { it.id == pending }
            if (idx >= 0) listState.scrollToItem(idx)
            model.pendingScrollTo = null
        }
    }

    Column(Modifier.fillMaxSize()) {
        HalogenNavbar(core = core, leading = {
            TextButton(onClick = {
                isEditing = !isEditing
                // Leaving Edit closes multi-select for real (the bulk bar is
                // keyed on a non-empty selection).
                if (!isEditing) selection = emptySet()
            }) {
                Text(if (isEditing) "Done" else "Edit")
            }
        })
        ListControlsBar(
            query = model.query,
            onQueryChange = { model.query = it },
            allowsOnDevice = !core.isEmbeddedAccount,
        )
        Box(Modifier.fillMaxSize().weight(1f)) {
            val error = model.error
            when {
                error != null -> LoadErrorView(
                    title = "Couldn't load latest",
                    message = error,
                    retry = { model.refresh() },
                )
                model.loaded && model.episodes.isEmpty() -> EmptyStateView(
                    systemImage = "clock",
                    title = "Nothing here",
                    description = if (model.query == ListQuery())
                        "New episodes land here after each feed poll."
                    else
                        "Nothing matches the current search/filter.",
                )
                else -> PullToRefreshBox(
                    isRefreshing = refreshing,
                    onRefresh = {
                        scope.launch {
                            refreshing = true
                            try {
                                model.refresh()
                            } finally {
                                refreshing = false
                            }
                        }
                    },
                ) {
                    LazyColumn(state = listState, modifier = Modifier.fillMaxSize()) {
                        items(count = displayed.size, key = { displayed[it].id }) { index ->
                            val episode = displayed[index]
                            DisposableEffect(episode.id) {
                                model.rowAppeared(episode)
                                onDispose { model.rowDisappeared(episode) }
                            }
                            SelectableEpisodeRow(
                                episode = episode,
                                subtitle = episode.podcast?.title,
                                page = SwipePage.Latest,
                                isEditing = isEditing,
                                selected = episode.id in selection,
                                onToggleSelect = {
                                    selection = if (episode.id in selection)
                                        selection - episode.id
                                    else selection + episode.id
                                },
                                core = core,
                            )
                        }
                        if (model.hasMore && model.episodes.isNotEmpty() && !selectedOnly) {
                            item {
                                LoadMoreRow(failed = model.loadMoreFailed) { model.loadMore() }
                            }
                        }
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
