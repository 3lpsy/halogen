package org.fgsec.halogen.features.podcasts

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.DropdownMenu
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
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch
import org.fgsec.halogen.components.AppRoute
import org.fgsec.halogen.components.Artwork
import org.fgsec.halogen.components.EmptyStateView
import org.fgsec.halogen.components.HalogenNavbar
import org.fgsec.halogen.components.LoadErrorView
import org.fgsec.halogen.components.LoadMoreRow
import org.fgsec.halogen.components.LocalNavigator
import org.fgsec.halogen.components.SortSearchBar
import org.fgsec.halogen.components.halogenIcon
import org.fgsec.halogen.core.HalogenCore
import org.fgsec.halogen.wire.PodcastData

/// The library tab: every subscribed podcast, local-first, artwork via the
/// server art cache. Rows carry the quick-actions ellipsis (open is the row
/// tap); management pushes via state-driven destinations.
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun PodcastsView(model: PodcastsModel, core: HalogenCore) {
    val navigator = LocalNavigator.current
    val scope = rememberCoroutineScope()
    var showCreate by remember { mutableStateOf(false) }
    var manageTarget by remember { mutableStateOf<PodcastManageTarget?>(null) }
    var deleteTarget by remember { mutableStateOf<PodcastData?>(null) }
    var refreshing by remember { mutableStateOf(false) }

    LaunchedEffect(Unit) { model.load() }

    // A library-row manage push (state-driven, like navigationDestination).
    manageTarget?.let { target ->
        val podcast = model.podcasts.firstOrNull { it.id == target.podcastId }
        if (podcast != null) {
            BackHandler { manageTarget = null }
            PodcastManageScreen(
                route = target.route, core = core, podcast = podcast,
                onBack = { manageTarget = null },
            )
            return
        }
    }

    Column(Modifier.fillMaxSize()) {
        HalogenNavbar(core = core) {
            IconButton(
                onClick = { showCreate = true },
                modifier = Modifier.testTag("podcast-add"),
            ) {
                Icon(halogenIcon("plus"), contentDescription = "Add podcast")
            }
        }
        SortSearchBar(
            search = model.query.search,
            onSearchChange = { model.query = model.query.copy(search = it) },
            field = model.query.field,
            onFieldChange = { model.query = model.query.copy(field = it) },
            direction = model.query.direction,
            onDirectionChange = { model.query = model.query.copy(direction = it) },
            fields = PodcastSortField.entries.map { it to it.label },
            placeholder = "Search podcasts",
        )
        Box(Modifier.fillMaxSize().weight(1f)) {
            val error = model.error
            when {
                error != null -> LoadErrorView(
                    title = "Couldn't load podcasts",
                    message = error,
                    retry = { model.refresh() },
                )
                model.loaded && model.podcasts.isEmpty() -> EmptyStateView(
                    systemImage = "waveform.circle",
                    title = "No podcasts yet",
                    description = "Subscribe from Discover, or add a feed URL with +.",
                )
                model.loaded && model.displayed.isEmpty() -> EmptyStateView(
                    systemImage = "magnifyingglass",
                    title = "No results for “${model.query.search}”",
                    description = "Check the spelling or try a new search.",
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
                    val displayed = model.displayed
                    LazyColumn(Modifier.fillMaxSize()) {
                        items(count = displayed.size, key = { displayed[it].id }) { index ->
                            val podcast = displayed[index]
                            PodcastLibraryRow(
                                podcast = podcast,
                                core = core,
                                onOpen = { navigator?.push(AppRoute.Podcast(podcast.id)) },
                                onManage = { route ->
                                    manageTarget = PodcastManageTarget(podcast.id, route)
                                },
                                onDelete = { deleteTarget = podcast },
                            )
                        }
                        // The sentinel pages the raw browse order; a live
                        // search shows every loaded match instead.
                        if (model.hasMore && model.podcasts.isNotEmpty() &&
                            model.query.search.isEmpty()
                        ) {
                            item {
                                LoadMoreRow(failed = model.loadMoreFailed) { model.loadMore() }
                            }
                        }
                    }
                }
            }
        }
    }

    if (showCreate) {
        PodcastCreateSheet(
            core = core,
            onDismiss = { showCreate = false },
            onCreated = { model.refresh() },
        )
    }

    deleteTarget?.let { target ->
        AlertDialog(
            onDismissRequest = { deleteTarget = null },
            title = { Text("Delete podcast?") },
            text = { Text("Removes the podcast, its episodes, and their server files.") },
            confirmButton = {
                TextButton(onClick = {
                    deleteTarget = null
                    // Durable unsubscribe (optimistic + outbox) — offline-safe.
                    scope.launch { core.unsubscribePodcast(target.id) }
                }) {
                    Text("Delete ${target.title}", color = MaterialTheme.colorScheme.error)
                }
            },
            dismissButton = {
                TextButton(onClick = { deleteTarget = null }) { Text("Cancel") }
            },
        )
    }
}

/// A library-row manage push: which podcast + which screen.
data class PodcastManageTarget(val podcastId: Int, val route: PodcastManageRoute)

/// One library row's tap targets: open (row) + the manage menu (ellipsis and
/// long-press mirror — same shared content, the two can't drift).
@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun PodcastLibraryRow(
    podcast: PodcastData,
    core: HalogenCore,
    onOpen: () -> Unit,
    onManage: (PodcastManageRoute) -> Unit,
    onDelete: () -> Unit,
) {
    var menuOpen by remember { mutableStateOf(false) }
    Row(
        Modifier
            .fillMaxWidth()
            .combinedClickable(onClick = onOpen, onLongClick = { menuOpen = true })
            .padding(start = 16.dp, end = 4.dp, top = 6.dp, bottom = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        PodcastRow(
            podcast = podcast,
            artUrl = core.podcastArtUrl(podcast),
            modifier = Modifier.weight(1f),
        )
        Box {
            IconButton(onClick = { menuOpen = true }) {
                Icon(
                    halogenIcon("ellipsis"),
                    contentDescription = "Podcast actions",
                    tint = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            DropdownMenu(expanded = menuOpen, onDismissRequest = { menuOpen = false }) {
                PodcastManageMenu(
                    onManage = {
                        menuOpen = false
                        onManage(it)
                    },
                    onDelete = {
                        menuOpen = false
                        onDelete()
                    },
                )
            }
        }
    }
}

/// One library row: artwork, title, author, episode count.
@Composable
fun PodcastRow(podcast: PodcastData, artUrl: String?, modifier: Modifier = Modifier) {
    Row(
        modifier,
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Artwork(url = artUrl, size = 56.dp)
        Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
            Text(
                podcast.title,
                style = MaterialTheme.typography.titleMedium,
                maxLines = 2,
                overflow = TextOverflow.Ellipsis,
            )
            val author = podcast.author
            if (!author.isNullOrEmpty()) {
                Text(
                    author,
                    style = MaterialTheme.typography.labelMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
            podcast.episode_count?.let { count ->
                Text(
                    "$count episodes",
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
    }
}
