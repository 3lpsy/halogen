package org.fgsec.halogen.features.podcasts

import org.fgsec.halogen.core.enqueueMutation
import org.fgsec.halogen.core.ensureQueuedBatch
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.MenuDefaults
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import kotlin.coroutines.cancellation.CancellationException
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.drop
import kotlinx.coroutines.launch
import org.fgsec.halogen.components.Artwork
import org.fgsec.halogen.components.ConfiguredSwipes
import org.fgsec.halogen.components.EmptyStateView
import org.fgsec.halogen.components.EpisodeMenuContext
import org.fgsec.halogen.components.EpisodeRowLink
import org.fgsec.halogen.components.HtmlText
import org.fgsec.halogen.components.ListControlsBar
import org.fgsec.halogen.components.ListQuery
import org.fgsec.halogen.components.ListQueryKeys
import org.fgsec.halogen.components.LoadErrorView
import org.fgsec.halogen.components.LoadMoreRow
import org.fgsec.halogen.components.LocalNavigator
import org.fgsec.halogen.components.halogenIcon
import org.fgsec.halogen.core.ConnectionMonitor
import org.fgsec.halogen.core.DeviceDownloads
import org.fgsec.halogen.core.DeviceLog
import org.fgsec.halogen.core.FriendlyError
import org.fgsec.halogen.core.HalogenCore
import org.fgsec.halogen.core.SwipePage
import org.fgsec.halogen.features.latest.EpisodeFilter
import org.fgsec.halogen.storage.CacheKey
import org.fgsec.halogen.storage.OutboxOp
import org.fgsec.halogen.wire.EpisodeData
import org.fgsec.halogen.wire.PodcastData

/// One podcast's episodes, newest first — the podcast's home. The toolbar
/// menu carries podcast management (edit / download config / auto-playlists /
/// metadata / delete); Edit mode enables multi-select with bulk actions.
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun EpisodesView(core: HalogenCore, podcast: PodcastData) {
    val navigator = LocalNavigator.current
    val scope = rememberCoroutineScope()
    val state = remember(podcast.id) { EpisodesScreenState(core, podcast) }
    var selection by remember { mutableStateOf(setOf<Int>()) }
    var selectedOnly by remember { mutableStateOf(false) }
    /// Row selection is an EDIT-MODE tool only — rows keep their normal taps
    /// until Edit is active; leaving Edit clears the set (closes the bulk bar).
    var isEditing by remember { mutableStateOf(false) }
    var confirmDelete by remember { mutableStateOf(false) }
    var manage by remember { mutableStateOf<PodcastManageRoute?>(null) }
    var manageMenuOpen by remember { mutableStateOf(false) }
    var refreshing by remember { mutableStateOf(false) }

    // task(id: query): first appearance and every query change reload.
    LaunchedEffect(state.query) { state.load() }

    // Screen-local state: not covered by resyncAfterReconnect — heal a stuck
    // error state when the network returns.
    LaunchedEffect(Unit) {
        snapshotFlow { core.connection.status }
            .drop(1)
            .collect { status ->
                if (status == ConnectionMonitor.Status.Online && state.error != null) {
                    state.load()
                }
            }
    }

    // OnDevice chip reactivity: recompute when the device set changes (see
    // LatestView's identical hook).
    LaunchedEffect(Unit) {
        snapshotFlow { core.models?.device?.onDevice?.map { it.id } ?: emptyList() }
            .drop(1)
            .collect { if (state.isDeviceSet) state.load() }
    }

    // A manage push (state-driven, like navigationDestination(item:)).
    manage?.let { route ->
        BackHandler { manage = null }
        PodcastManageScreen(
            route = route, core = core, podcast = podcast,
            onBack = { manage = null },
        )
        return
    }

    /// The "Selected" review chip restricts rows to the ticked set.
    val displayed =
        if (selectedOnly) state.episodes.filter { it.id in selection } else state.episodes

    Column(Modifier.fillMaxSize()) {
        TopAppBar(
            title = {
                Text(podcast.title, maxLines = 1, overflow = TextOverflow.Ellipsis)
            },
            navigationIcon = {
                IconButton(onClick = { navigator?.pop() }) {
                    Icon(halogenIcon("chevron.left"), contentDescription = "Back")
                }
            },
            actions = {
                Box {
                    IconButton(onClick = { manageMenuOpen = true }) {
                        Icon(
                            halogenIcon("ellipsis.circle"),
                            contentDescription = "Manage podcast",
                        )
                    }
                    DropdownMenu(
                        expanded = manageMenuOpen,
                        onDismissRequest = { manageMenuOpen = false },
                    ) {
                        PodcastManageMenu(
                            onManage = {
                                manageMenuOpen = false
                                manage = it
                            },
                            onDelete = {
                                manageMenuOpen = false
                                confirmDelete = true
                            },
                        )
                    }
                }
                TextButton(onClick = {
                    isEditing = !isEditing
                    // Leaving Edit closes multi-select for real (the bulk bar
                    // is keyed on a non-empty selection).
                    if (!isEditing) selection = emptySet()
                }) {
                    Text(if (isEditing) "Done" else "Edit")
                }
            },
        )
        ListControlsBar(
            query = state.query,
            onQueryChange = { state.query = it },
            allowsOnDevice = !core.isEmbeddedAccount,
        )
        Box(Modifier.fillMaxSize().weight(1f)) {
            val error = state.error
            val refresh = {
                scope.launch {
                    refreshing = true
                    try {
                        state.load()
                    } finally {
                        refreshing = false
                    }
                }
                Unit
            }
            when {
                error != null -> LoadErrorView(
                    title = "Couldn't load episodes",
                    message = error,
                    retry = { state.load() },
                )
                state.loaded && state.episodes.isEmpty() ->
                    // "No episodes yet" over an active search/filter implied
                    // an unpolled feed — distinguish no-match from truly
                    // empty. Refreshable: "appears after the next poll" is an
                    // invitation to pull once the poll has run.
                    PullToRefreshBox(isRefreshing = refreshing, onRefresh = refresh) {
                        if (state.query.search.isNotEmpty() || state.query.filters.isNotEmpty()) {
                            EmptyStateView(
                                systemImage = "line.3.horizontal.decrease.circle",
                                title = "No matches",
                                description = "No episodes match the current search or filters.",
                            )
                        } else {
                            EmptyStateView(
                                systemImage = "waveform.circle",
                                title = "No episodes yet",
                                description = "Episodes appear after the next feed poll.",
                            )
                        }
                    }
                else -> PullToRefreshBox(
                    isRefreshing = refreshing,
                    onRefresh = refresh,
                ) {
                    LazyColumn(Modifier.fillMaxSize()) {
                        // The podcast's header block (art, author, description)
                        // — the web detail's identity strip; from the already-
                        // loaded row, no extra fetch.
                        item(key = "header") {
                            Row(
                                Modifier.fillMaxWidth().padding(16.dp),
                                horizontalArrangement = Arrangement.spacedBy(14.dp),
                            ) {
                                Artwork(
                                    url = core.podcastArtUrl(podcast, small = false),
                                    size = 88.dp,
                                )
                                Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                                    Text(
                                        podcast.title,
                                        style = MaterialTheme.typography.titleMedium,
                                    )
                                    val author = podcast.author
                                    if (!author.isNullOrEmpty()) {
                                        Text(
                                            author,
                                            style = MaterialTheme.typography.labelMedium,
                                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                                        )
                                    }
                                    if (podcast.description.isNotEmpty()) {
                                        Text(
                                            HtmlText.preview(podcast.description),
                                            style = MaterialTheme.typography.labelSmall,
                                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                                            maxLines = 3,
                                            overflow = TextOverflow.Ellipsis,
                                        )
                                    }
                                }
                            }
                        }
                        items(count = displayed.size, key = { displayed[it].id }) { index ->
                            val episode = displayed[index]
                            SelectableEpisodeRow(
                                episode = episode,
                                page = SwipePage.PodcastEpisodes,
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
                        if (state.hasMore && state.episodes.isNotEmpty() && !selectedOnly) {
                            item {
                                LoadMoreRow(failed = state.loadMoreFailed) { state.loadMore() }
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
                episodes = state.episodes,
                core = core,
            )
        }
    }

    if (confirmDelete) {
        AlertDialog(
            onDismissRequest = { confirmDelete = false },
            title = { Text("Delete podcast?") },
            text = { Text("Removes the podcast, its episodes, and their server files.") },
            confirmButton = {
                TextButton(onClick = {
                    confirmDelete = false
                    scope.launch {
                        // Durable unsubscribe (optimistic + outbox) — works offline.
                        core.unsubscribePodcast(podcast.id)
                        navigator?.pop()
                    }
                }) {
                    Text("Delete ${podcast.title}", color = MaterialTheme.colorScheme.error)
                }
            },
            dismissButton = {
                TextButton(onClick = { confirmDelete = false }) { Text("Cancel") }
            },
        )
    }
}

/// The screen's view-state — deliberately view-local (unlike Latest), so a
/// fresh push starts a fresh load cycle; `remember(podcast.id)` scopes it.
private class EpisodesScreenState(
    private val core: HalogenCore,
    private val podcast: PodcastData,
) {
    private val accountStore = core.store

    var episodes by mutableStateOf<List<EpisodeData>>(emptyList())
    var error by mutableStateOf<String?>(null)
    var loaded by mutableStateOf(false)
    var hasMore by mutableStateOf(false)
    var loadMoreFailed by mutableStateOf(false)
    var query by mutableStateOf(ListQuery())

    private var loadingMore = false
    private var page = 0
    private var generation = 0
    private var loadedQuery = false

    private val cacheKey: String get() = CacheKey.podcastEpisodes(podcast.id)

    private val isDefaultQuery: Boolean get() = query == ListQuery()

    /// OnDevice chip (non-embedded): the list becomes this podcast's slice of
    /// the local device set (web: OnDevice branch scoped by podcast_id).
    val isDeviceSet: Boolean
        get() = query.filters.contains(EpisodeFilter.OnDevice) && !core.isEmbeddedAccount

    /// Local-first: the cached first page renders before the network answers
    /// (canonical query only), then a fresh page 0 replaces + re-snapshots.
    suspend fun load() {
        // Restore/persist the shared podcast-episodes list state (web:
        // use_list_view_state("podcast") — one key across all podcasts).
        if (!loadedQuery) {
            loadedQuery = true
            val saved = accountStore?.load<ListQuery>(ListQueryKeys.podcastEpisodes)
            if (saved != null && saved != query) {
                // Adopt and let LaunchedEffect(query) refire for the restored value.
                query = saved
                return
            }
        }
        val snapshot = query
        core.scope.launch { accountStore?.save(snapshot, ListQueryKeys.podcastEpisodes) }
        if (query.search.isNotEmpty()) delay(300)
        if (isDeviceSet) {
            val device = core.models?.device
            val all = (device?.onDevice ?: emptyList()).filter {
                it.podcast_id == podcast.id &&
                    device?.stateOf(it.id) == DeviceDownloads.State.Downloaded
            }
            episodes = query.apply(all, status = { core.overlayStatus(it) })
            hasMore = false
            page = 0
            error = null
            loaded = true
            return
        }
        if (episodes.isEmpty() && isDefaultQuery) {
            accountStore?.load<List<EpisodeData>>(cacheKey)?.let {
                episodes = it
                loaded = true
            }
        }
        generation += 1
        val mine = generation
        try {
            val first = core.episodes(podcast.id, extra = query.queryItems, page = 0)
            if (mine != generation) return
            episodes = filteredForChips(first.items)
            hasMore = first.hasMore
            page = 0
            error = null
            if (isDefaultQuery) {
                // Persist WITHOUT shrinking back to one page: fresh page 0
                // leads, previously cached rows keep the tail — pages the
                // user scrolled through stay renderable offline.
                val ids = first.items.map { it.id }.toSet()
                var snap = first.items
                accountStore?.load<List<EpisodeData>>(cacheKey)?.let { prior ->
                    snap = snap + prior.filter { it.id !in ids }
                }
                accountStore?.save(snap, cacheKey)
            }
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            DeviceLog.warn("episodes: refresh failed — ${e::class.simpleName}: ${e.message}")
            if (mine != generation) return
            if (episodes.isEmpty()) error = FriendlyError.message(e)
        }
        loaded = true
    }

    /// Multi-chip facets can't ride the wire — trim each fetched page locally.
    private fun filteredForChips(items: List<EpisodeData>): List<EpisodeData> =
        if (query.needsLocalChipFilter)
            items.filter { query.matchesChips(it, status = { e -> core.overlayStatus(e) }) }
        else items

    suspend fun loadMore() {
        if (!hasMore || loadingMore || !loaded || isDeviceSet) return
        loadingMore = true
        loadMoreFailed = false
        // Same generation guard as load(): a query change mid-fetch must drop
        // this page, not append the old query's rows (and persist them).
        val mine = generation
        try {
            val next = core.episodes(podcast.id, extra = query.queryItems, page = page + 1)
            if (mine != generation) return
            page += 1
            val known = episodes.map { it.id }.toSet()
            episodes = episodes + filteredForChips(next.items.filter { it.id !in known })
            hasMore = next.hasMore
            if (isDefaultQuery) {
                // Extend the offline snapshot with the appended page.
                accountStore?.save(episodes, cacheKey)
            }
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            DeviceLog.warn("episodes: loadMore failed — ${e::class.simpleName}: ${e.message}")
            loadMoreFailed = true
        } finally {
            loadingMore = false
        }
    }
}

/// One selectable episode row: normal mode wraps the shared row in the
/// configured swipe container; Edit mode swaps swipes for a selection tick
/// and hijacks the row tap into select-toggle (iOS List edit mode).
@Composable
fun SelectableEpisodeRow(
    episode: EpisodeData,
    page: SwipePage,
    isEditing: Boolean,
    selected: Boolean,
    onToggleSelect: () -> Unit,
    core: HalogenCore,
    subtitle: String? = null,
) {
    if (isEditing) {
        Box {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Icon(
                    halogenIcon(if (selected) "checkmark.circle.fill" else "circle"),
                    contentDescription = if (selected) "Selected" else "Not selected",
                    tint = if (selected) MaterialTheme.colorScheme.primary
                    else MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.padding(start = 16.dp).size(22.dp),
                )
                Box(Modifier.weight(1f)) {
                    EpisodeRowLink(
                        episode = episode,
                        artUrl = core.episodeArtUrl(episode),
                        subtitle = subtitle,
                        context = EpisodeMenuContext.Browse,
                        core = core,
                    )
                }
            }
            Box(
                Modifier
                    .matchParentSize()
                    .clickable(
                        interactionSource = remember { MutableInteractionSource() },
                        indication = null,
                    ) { onToggleSelect() }
            )
        }
    } else {
        ConfiguredSwipes(page = page, episode = episode, core = core) {
            EpisodeRowLink(
                episode = episode,
                artUrl = core.episodeArtUrl(episode),
                subtitle = subtitle,
                context = EpisodeMenuContext.Browse,
                core = core,
            )
        }
    }
}

/// Bulk actions over an edit-mode multi-selection — the web bulk menu's full section
/// set (bulk_menu.rs): queue/playlist membership, server + device download variants,
/// the "Selected" review chip, "Select all"; durable actions queue per id via the outbox.
@Composable
fun BulkActionBar(
    selection: Set<Int>,
    onSelectionChange: (Set<Int>) -> Unit,
    selectedOnly: Boolean,
    onSelectedOnlyChange: (Boolean) -> Unit,
    episodes: List<EpisodeData>,
    core: HalogenCore,
    context: EpisodeMenuContext = EpisodeMenuContext.Browse,
) {
    var menuOpen by remember { mutableStateOf(false) }
    // Leaving the bar drops the review restriction (web parity).
    DisposableEffect(Unit) { onDispose { onSelectedOnlyChange(false) } }

    fun selectedEpisodes(): List<EpisodeData> = episodes.filter { it.id in selection }

    fun forEachSelected(action: (EpisodeData) -> Unit) {
        for (episode in episodes) {
            if (episode.id in selection) action(episode)
        }
        onSelectionChange(emptySet())
    }

    Column(Modifier.fillMaxWidth()) {
        HorizontalDivider()
        Row(
            Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 10.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            Text(
                "${selection.size} selected",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            OutlinedButton(
                onClick = { onSelectedOnlyChange(!selectedOnly) },
                colors = ButtonDefaults.outlinedButtonColors(
                    contentColor = if (selectedOnly) MaterialTheme.colorScheme.primary
                    else MaterialTheme.colorScheme.onSurfaceVariant,
                ),
            ) {
                Text("Selected", style = MaterialTheme.typography.bodySmall)
            }
            OutlinedButton(
                onClick = {
                    // Selects the LOADED rows only (what's fetched so far) —
                    // rows loaded afterwards are not auto-selected (web rule).
                    onSelectionChange(episodes.map { it.id }.toSet())
                },
                colors = ButtonDefaults.outlinedButtonColors(
                    contentColor = MaterialTheme.colorScheme.onSurfaceVariant,
                ),
            ) {
                Text("Select all", style = MaterialTheme.typography.bodySmall)
            }
            Spacer(Modifier.weight(1f))
            Box {
                IconButton(onClick = { menuOpen = true }) {
                    Icon(halogenIcon("ellipsis.circle"), contentDescription = "Bulk actions")
                }
                DropdownMenu(expanded = menuOpen, onDismissRequest = { menuOpen = false }) {
                    BulkActionsMenu(
                        core = core,
                        context = context,
                        selectedEpisodes = ::selectedEpisodes,
                        forEachSelected = { action ->
                            menuOpen = false
                            forEachSelected(action)
                        },
                        onSelectionChange = {
                            menuOpen = false
                            onSelectionChange(it)
                        },
                    )
                }
            }
        }
    }
}

/// The bulk menu's items (sections divider-separated; destructive rows in
/// the error tint) — mirrors the Swift actionsMenu exactly.
@Composable
private fun BulkActionsMenu(
    core: HalogenCore,
    context: EpisodeMenuContext,
    selectedEpisodes: () -> List<EpisodeData>,
    forEachSelected: ((EpisodeData) -> Unit) -> Unit,
    onSelectionChange: (Set<Int>) -> Unit,
) {
    val embedded = core.isEmbeddedAccount
    val destructive = MenuDefaults.itemColors(
        textColor = MaterialTheme.colorScheme.error,
        leadingIconColor = MaterialTheme.colorScheme.error,
    )

    DropdownMenuItem(
        text = { Text("Add to queue") },
        leadingIcon = { Icon(halogenIcon("text.badge.plus"), null) },
        onClick = { forEachSelected { core.models?.queue?.add(it) } },
    )
    DropdownMenuItem(
        text = { Text("Remove from queue") },
        leadingIcon = { Icon(halogenIcon("text.badge.minus"), null) },
        onClick = { forEachSelected { core.models?.queue?.remove(it) } },
    )
    HorizontalDivider()
    DropdownMenuItem(
        text = { Text("Add to playlist") },
        leadingIcon = { Icon(halogenIcon("music.note.list"), null) },
        onClick = {
            // The shared picker dialog (RootView) resolves the target.
            core.models?.playlists?.requestPick(selectedEpisodes())
            onSelectionChange(emptySet())
        },
    )
    // Remove from the CURRENT playlist — playlist views that aren't the
    // queue (the queue section covers queue removal).
    if (context is EpisodeMenuContext.Playlist) {
        DropdownMenuItem(
            text = { Text("Remove from ${context.name}") },
            leadingIcon = { Icon(halogenIcon("minus.circle"), null) },
            colors = destructive,
            onClick = { forEachSelected { context.model.remove(it) } },
        )
    }
    HorizontalDivider()
    // Server tier — embedded drops the qualifier (nothing is remote).
    DropdownMenuItem(
        text = { Text(if (embedded) "Re-download" else "Re-download on server") },
        leadingIcon = { Icon(halogenIcon("arrow.triangle.2.circlepath"), null) },
        onClick = {
            // Remove-then-trigger in outbox order (web RedownloadOnServer).
            forEachSelected { episode ->
                core.scope.launch {
                    if (!core.ensureQueuedBatch(listOf(OutboxOp.Kind.RemoveServerDownload(episode.id), OutboxOp.Kind.TriggerDownload(episode.id)))) return@launch
                    core.models?.serverDownloads?.watch(episode.id)
                }
            }
        },
    )
    DropdownMenuItem(
        text = { Text(if (embedded) "Download" else "Download on server") },
        leadingIcon = {
            Icon(halogenIcon(if (embedded) "arrow.down.circle" else "icloud.and.arrow.down"), null)
        },
        onClick = {
            // Durable trigger + progress tracking (queues offline instead of
            // silently dropping the batch).
            forEachSelected { core.models?.serverDownloads?.download(it) }
        },
    )
    DropdownMenuItem(
        text = { Text(if (embedded) "Remove download" else "Remove from server") },
        leadingIcon = { Icon(halogenIcon(if (embedded) "trash" else "icloud.slash"), null) },
        colors = destructive,
        onClick = {
            forEachSelected { episode ->
                core.enqueueMutation(OutboxOp.Kind.RemoveServerDownload(episode.id)) {
                    core.models?.serverDownloads?.markRemovedLocally(episode.id)
                }
            }
        },
    )
    if (!embedded) {
        HorizontalDivider()
        DropdownMenuItem(
            text = { Text("Re-download on device") },
            leadingIcon = { Icon(halogenIcon("arrow.clockwise.circle"), null) },
            onClick = {
                forEachSelected { episode ->
                    core.models?.device?.remove(episode.id)
                    core.models?.device?.download(episode)
                }
            },
        )
        DropdownMenuItem(
            text = { Text("Download to device") },
            leadingIcon = { Icon(halogenIcon("arrow.down.to.line.circle"), null) },
            onClick = { forEachSelected { core.models?.device?.download(it) } },
        )
        DropdownMenuItem(
            text = { Text("Remove from device") },
            leadingIcon = { Icon(halogenIcon("iphone.slash"), null) },
            colors = destructive,
            onClick = { forEachSelected { core.models?.device?.remove(it.id) } },
        )
    }
}
