package org.fgsec.halogen.features.playlists

import org.fgsec.halogen.core.enqueueMutation
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
import androidx.compose.material3.CenterAlignedTopAppBar
import androidx.compose.material3.Checkbox
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
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
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.launch
import org.fgsec.halogen.components.ConfiguredSwipeRow
import org.fgsec.halogen.components.EmptyStateView
import org.fgsec.halogen.components.EpisodeMenuContext
import org.fgsec.halogen.components.EpisodeOrderField
import org.fgsec.halogen.components.EpisodeRowLink
import org.fgsec.halogen.components.ListControlsBar
import org.fgsec.halogen.components.ListQuery
import org.fgsec.halogen.components.LoadErrorView
import org.fgsec.halogen.features.podcasts.BulkActionBar
import org.fgsec.halogen.components.RowSwipeAction
import org.fgsec.halogen.components.halogenIcon
import org.fgsec.halogen.components.rememberReorderableListState
import org.fgsec.halogen.components.reorderHandle
import org.fgsec.halogen.components.reorderableItem
import org.fgsec.halogen.core.ConnectionMonitor
import org.fgsec.halogen.core.DeviceDownloads
import org.fgsec.halogen.core.DeviceLog
import org.fgsec.halogen.core.FriendlyError
import org.fgsec.halogen.core.HalogenCore
import org.fgsec.halogen.core.SwipePage
import org.fgsec.halogen.storage.CacheKey
import org.fgsec.halogen.storage.OutboxOp
import org.fgsec.halogen.wire.EpisodeData
import org.fgsec.halogen.wire.OrderDirection
import org.fgsec.halogen.wire.PlaylistData

/// One playlist's episodes in position order — same offline-capable
/// remove/reorder as the queue (they share the outbox op vocabulary and the
/// per-playlist episode cache key).
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun PlaylistDetailView(
    core: HalogenCore,
    playlist: PlaylistData,
    onBack: (() -> Unit)? = null,
) {
    val model = remember(playlist.id) { PlaylistEpisodesModel(core, playlist.id) }
    val context = EpisodeMenuContext.Playlist(name = playlist.name, model = model)
    // Row selection is an EDIT-MODE tool only (queue rule): rows navigate
    // until Edit is active, and leaving Edit clears the set (closes the
    // bulk bar).
    var editing by remember { mutableStateOf(false) }
    var selection by remember { mutableStateOf(setOf<Int>()) }
    var selectedOnly by remember { mutableStateOf(false) }
    var refreshing by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()

    LaunchedEffect(Unit) { model.load() }
    // Screen-local model: not covered by resyncAfterReconnect — heal a
    // stuck error state when the network returns.
    LaunchedEffect(core.connection.status) {
        if (core.connection.status == ConnectionMonitor.Status.Online && model.error != null) {
            model.refresh()
        }
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
        CenterAlignedTopAppBar(
            title = { Text(playlist.name) },
            navigationIcon = {
                if (onBack != null) {
                    IconButton(onClick = onBack) {
                        Icon(halogenIcon("chevron.left"), contentDescription = "Back")
                    }
                }
            },
            actions = {
                TextButton(onClick = { editing = !editing }) {
                    Text(if (editing) "Done" else "Edit")
                }
            },
        )
        ListControlsBar(
            query = model.query,
            onQueryChange = { model.query = it },
            allowsPosition = true,
            allowsOnDevice = !core.isEmbeddedAccount,
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
                        LoadErrorView(title = "Couldn't load playlist", message = error) {
                            model.refresh()
                        }

                    model.loaded && model.episodes.isEmpty() ->
                        EmptyStateView(
                            systemImage = "music.note.list",
                            title = "Empty playlist",
                            description = "Add episodes from any list via their context menu.",
                        )

                    model.loaded && model.displayed.isEmpty() ->
                        EmptyStateView(
                            systemImage = "line.3.horizontal.decrease.circle",
                            title = "No matches",
                            description = "No episodes match the current search or filters.",
                        )

                    else -> PlaylistEpisodeList(
                        model = model, core = core, context = context,
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
                context = context,
            )
        }
    }
}

@Composable
private fun PlaylistEpisodeList(
    model: PlaylistEpisodesModel,
    core: HalogenCore,
    context: EpisodeMenuContext,
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
        // Manual reorder only over the raw position order with no
        // search/filter (queue rule; web parity). selectedOnly shows a
        // SUBSET — its indices don't map onto the full array, and a drag
        // would silently reorder hidden rows and sync that corruption.
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
                    page = SwipePage.Playlist,
                    episode = episode,
                    core = core,
                    context = context,
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
                                context = context,
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

/// Episodes-of-a-playlist state, shared semantics with the queue (which is
/// just the default playlist): cache-first render, optimistic outbox ops.
class PlaylistEpisodesModel(
    private val core: HalogenCore,
    /// Exposed so play actions can carry this list as the play context.
    val playlistId: Int,
) {
    private val accountStore = core.store

    var episodes: List<EpisodeData> by mutableStateOf(emptyList())
        private set
    var loaded: Boolean by mutableStateOf(false)
        private set
    var error: String? by mutableStateOf(null)
        private set
    private var loadedQuery = false
    /// Bumped synchronously by every optimistic mutation; a refresh discards
    /// its response if this moved mid-fetch (see QueueModel.mutationEpoch).
    private var mutationEpoch = 0

    /// Search + chips + order, applied locally over the position-ordered
    /// membership (same as the queue — this list IS a playlist).
    private val queryState = mutableStateOf(defaultQuery)
    var query: ListQuery
        get() = queryState.value
        set(value) {
            if (value == queryState.value) return
            queryState.value = value
            val store = accountStore
            core.scope.launch { store?.save(value, QUERY_KEY) }
        }

    /// The rows the view renders (query over the raw membership).
    val displayed: List<EpisodeData>
        get() = query.apply(
            episodes,
            isOnDevice = { id ->
                core.models?.device?.stateOf(id) == DeviceDownloads.State.Downloaded
            },
            status = { core.overlayStatus(it) },
        )

    /// Reordering is only meaningful over the raw position order with no
    /// search/filter applied (queue rule).
    val reorderable: Boolean
        get() = query == defaultQuery

    suspend fun load() {
        error = null
        if (!loadedQuery) {
            loadedQuery = true
            accountStore?.load<ListQuery>(QUERY_KEY)?.let { queryState.value = it }
        }
        if (episodes.isEmpty()) {
            accountStore?.load<List<EpisodeData>>(CacheKey.playlistEpisodes(playlistId))?.let {
                episodes = it
                loaded = true
            }
        }
        refresh()
    }

    suspend fun refresh() {
        // Drain BEFORE pulling, and keep the local list when membership ops
        // are still queued — same rationale as QueueModel.refresh (the queue
        // is just the default playlist).
        core.outbox?.drain()
        if (core.outbox?.hasPendingOps(playlistId) == true) {
            loaded = true
            return
        }
        try {
            val epoch = mutationEpoch
            val fresh = core.playlistEpisodes(playlistId)
            // Guard AGAIN at cache time: a mutation that landed while the
            // fetch was in flight outranks the stale server membership.
            if (epoch != mutationEpoch) {
                loaded = true
                return
            }
            if (core.outbox?.hasPendingOps(playlistId) == true) {
                loaded = true
                return
            }
            episodes = fresh
            error = null
            accountStore?.save(fresh, CacheKey.playlistEpisodes(playlistId))
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            DeviceLog.warn("playlist-detail: refresh failed — ${e::class.simpleName}: ${e.message}")
            if (episodes.isEmpty()) error = FriendlyError.message(e)
        }
        loaded = true
    }

    fun remove(episode: EpisodeData) {
        core.enqueueMutation(OutboxOp.Kind.RemoveFromPlaylist(playlistId = playlistId, episodeId = episode.id)) {
            mutationEpoch += 1
            episodes = episodes.filterNot { it.id == episode.id }
            core.models?.playlists?.setMembership(playlistId = playlistId, episodeIds = episodes.map { it.id })
        }
    }

    /// `toIndex` is the row's FINAL resting index (reorder helper semantics).
    fun move(fromIndex: Int, toIndex: Int) {
        val moved = episodes.getOrNull(fromIndex) ?: return
        val finalIndex = toIndex.coerceIn(0, (episodes.size - 1).coerceAtLeast(0))
        core.enqueueMutation(OutboxOp.Kind.MoveInPlaylist(playlistId = playlistId, episodeId = moved.id, to = finalIndex)) {
            mutationEpoch += 1
            val list = episodes.filterNot { it.id == moved.id }.toMutableList()
            list.add(finalIndex.coerceIn(0, list.size), moved)
            episodes = list
            core.models?.playlists?.setMembership(playlistId = playlistId, episodeIds = episodes.map { it.id })
        }
    }



    private companion object {
        /// One shared query across every playlist detail (web: the
        /// "playlist" list-view key is shared, default Custom/position order).
        const val QUERY_KEY = "listquery-playlist"
        val defaultQuery =
            ListQuery(orderField = EpisodeOrderField.Position, direction = OrderDirection.Asc)
    }
}
