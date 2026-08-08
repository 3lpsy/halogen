package org.fgsec.halogen.features.queue

import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.DragHandle
import androidx.compose.material3.Button
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
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch
import org.fgsec.halogen.components.ConfiguredSwipeRow
import org.fgsec.halogen.components.EmptyStateView
import org.fgsec.halogen.components.EpisodeMenuContext
import org.fgsec.halogen.components.EpisodeRowLink
import org.fgsec.halogen.components.HalogenNavbar
import org.fgsec.halogen.components.ListControlsBar
import org.fgsec.halogen.components.LoadErrorView
import org.fgsec.halogen.features.podcasts.BulkActionBar
import org.fgsec.halogen.components.RowSwipeAction
import org.fgsec.halogen.components.rememberReorderableListState
import org.fgsec.halogen.components.reorderHandle
import org.fgsec.halogen.components.reorderableItem
import org.fgsec.halogen.core.HalogenCore
import org.fgsec.halogen.core.SwipePage

/// The Queue tab: the default playlist in position order. Drag to reorder
/// (Edit mode), swipe to remove — both offline-capable (optimistic + outbox).
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun QueueView(model: QueueModel, core: HalogenCore) {
    // Row selection is an EDIT-MODE tool only: plain row taps stay
    // navigation until Edit is active, and leaving Edit clears the set
    // (closes the bulk bar) — iOS editMode parity.
    var editing by remember { mutableStateOf(false) }
    var selection by remember { mutableStateOf(setOf<Int>()) }
    var selectedOnly by remember { mutableStateOf(false) }
    var refreshing by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()

    LaunchedEffect(Unit) { model.load() }
    // Leaving Edit closes multi-select for real (the bulk bar is keyed on a
    // non-empty selection).
    LaunchedEffect(editing) {
        if (!editing) {
            selection = emptySet()
            selectedOnly = false
        }
    }

    Column(Modifier.fillMaxSize()) {
        HalogenNavbar(core = core, title = "Queue", leading = {
            TextButton(onClick = { editing = !editing }) {
                Text(if (editing) "Done" else "Edit")
            }
        })
        ListControlsBar(
            query = model.query,
            onQueryChange = { model.query = it },
            allowsPosition = true,
        )
        Box(Modifier.weight(1f)) {
            PullToRefreshBox(
                isRefreshing = refreshing,
                onRefresh = {
                    scope.launch {
                        refreshing = true
                        model.refresh()
                        refreshing = false
                    }
                },
                modifier = Modifier.fillMaxSize(),
            ) {
                val error = model.error
                when {
                    error != null ->
                        // Cold offline start with nothing cached gets the
                        // web's dedicated tri-state copy, not a raw error.
                        if (core.isOffline) {
                            EmptyStateView(
                                systemImage = "wifi.slash",
                                title = "Queue unavailable offline",
                                description = "Reconnect once to load it — afterwards the queue stays available offline.",
                            )
                        } else {
                            LoadErrorView(title = "Couldn't load queue", message = error) {
                                model.refresh()
                            }
                        }

                    model.loaded && model.queue == null ->
                        NoQueue(model = model, core = core)

                    model.loaded && model.displayed.isEmpty() ->
                        EmptyStateView(
                            systemImage = "list.bullet",
                            title = if (model.episodes.isEmpty()) "Queue is empty" else "No matches",
                            description = "Add episodes from any list via their context menu.",
                        )

                    else -> QueueList(
                        model = model, core = core,
                        editing = editing,
                        selection = selection,
                        onSelectionChange = { selection = it },
                        selectedOnly = selectedOnly,
                    )
                }
            }
        }
        if (selection.isNotEmpty()) {
            BulkActionBar(
                selection = selection,
                onSelectionChange = { selection = it },
                selectedOnly = selectedOnly,
                onSelectedOnlyChange = { selectedOnly = it },
                episodes = model.displayed,
                core = core,
                context = EpisodeMenuContext.Queue,
            )
        }
    }
}

@Composable
private fun QueueList(
    model: QueueModel,
    core: HalogenCore,
    editing: Boolean,
    selection: Set<Int>,
    onSelectionChange: (Set<Int>) -> Unit,
    selectedOnly: Boolean,
) {
    // The "Selected" review chip restricts rows to the ticked set.
    val displayedEpisodes =
        if (selectedOnly && selection.isNotEmpty()) model.displayed.filter { it.id in selection }
        else model.displayed
    val listState = rememberLazyListState()
    val reorder = rememberReorderableListState(listState) { from, to ->
        // selectedOnly shows a SUBSET — its indices don't map onto the full
        // array, and a drag would silently reorder hidden rows and sync that
        // corruption to the server.
        if (model.reorderable && !selectedOnly) model.move(from, to)
    }
    val canReorder = editing && model.reorderable && !selectedOnly

    LazyColumn(state = listState, modifier = Modifier.fillMaxSize()) {
        itemsIndexed(displayedEpisodes, key = { _, episode -> episode.id }) { index, episode ->
            fun toggle() {
                onSelectionChange(
                    if (episode.id in selection) selection - episode.id else selection + episode.id)
            }
            Box(Modifier.reorderableItem(reorder, index)) {
                ConfiguredSwipeRow(
                    page = SwipePage.Queue,
                    episode = episode,
                    core = core,
                    context = EpisodeMenuContext.Queue,
                    extraTrailing = RowSwipeAction(
                        label = "Remove", systemImage = "minus.circle", destructive = true,
                    ) { model.remove(episode) },
                ) {
                    Row(
                        Modifier.fillMaxWidth(),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        if (editing) {
                            Checkbox(
                                checked = episode.id in selection,
                                onCheckedChange = { toggle() },
                            )
                        }
                        Box(Modifier.weight(1f)) {
                            EpisodeRowLink(
                                episode = episode,
                                artUrl = core.episodeArtUrl(episode),
                                subtitle = episode.podcast?.title,
                                context = EpisodeMenuContext.Queue,
                                core = core,
                            )
                            if (editing) {
                                // Edit mode: row taps toggle selection
                                // instead of navigating.
                                Box(
                                    Modifier.matchParentSize().clickable(
                                        interactionSource = remember { MutableInteractionSource() },
                                        indication = null,
                                    ) { toggle() }
                                )
                            }
                        }
                        if (canReorder) {
                            Icon(
                                Icons.Rounded.DragHandle,
                                contentDescription = "Reorder",
                                tint = MaterialTheme.colorScheme.onSurfaceVariant,
                                modifier = Modifier
                                    .reorderHandle(reorder, episode.id)
                                    .padding(horizontal = 12.dp),
                            )
                        }
                    }
                }
            }
        }
    }
}

/// Mirrors the web's "No queue yet" page: create the default playlist.
@Composable
private fun NoQueue(model: QueueModel, core: HalogenCore) {
    val scope = rememberCoroutineScope()
    EmptyStateView(
        systemImage = "list.bullet",
        title = "No queue yet",
        description = "The queue is your default playlist — create it to start queueing episodes.",
    ) {
        // Online-only: the created id anchors every later offline op
        // (web: the create form's submit is disabled offline).
        Button(
            onClick = { scope.launch { model.createQueue() } },
            enabled = !core.isOffline,
        ) {
            Text(if (core.isOffline) "Create queue (offline)" else "Create queue")
        }
    }
}
