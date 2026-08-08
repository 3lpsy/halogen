package org.fgsec.halogen.features.latest

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import kotlin.coroutines.cancellation.CancellationException
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import org.fgsec.halogen.components.EpisodeOrderField
import org.fgsec.halogen.components.ListQuery
import org.fgsec.halogen.components.ListQueryKeys
import org.fgsec.halogen.core.DeviceDownloads
import org.fgsec.halogen.core.DeviceLog
import org.fgsec.halogen.core.FriendlyError
import org.fgsec.halogen.core.HalogenCore
import org.fgsec.halogen.networking.HalogenClient
import org.fgsec.halogen.storage.CacheKey
import org.fgsec.halogen.wire.EpisodeData
import org.fgsec.halogen.wire.OrderDirection

/// View-state for the Latest tab: local-first (cached snapshot renders before
/// the network answers), paginated append, and a persisted scroll anchor.
/// Lives OUTSIDE the view (owned by the models registry), so tab switches and
/// navigation pops come back to the exact list + scroll state.
class LatestModel(private val core: HalogenCore) {

    var episodes by mutableStateOf<List<EpisodeData>>(emptyList())
        private set
    var hasMore by mutableStateOf(false)
        private set
    /// Last loadMore failed — the sentinel renders a retry (LoadMoreRow).
    var loadMoreFailed by mutableStateOf(false)
        private set
    var isLoadingMore by mutableStateOf(false)
        private set
    var loaded by mutableStateOf(false)
        private set
    var error by mutableStateOf<String?>(null)
        private set

    /// One-shot scroll-restore target — the view scrolls to it and clears it.
    var pendingScrollTo by mutableStateOf<Int?>(null)

    /// Search + facet + order (the persistent list-controls bar).
    var query by mutableStateOf(ListQuery())

    private var page = 0
    private var generation = 0
    private var restoredAnchor = false
    /// Row ids currently on screen (row enter/leave composition) — the
    /// topmost one is the scroll anchor persisted across relaunches.
    private val visible = mutableSetOf<Int>()
    private var loadedQuery = false

    /// First appearance / query change: cached snapshot first, then refresh.
    suspend fun load() {
        error = null
        if (!loadedQuery) {
            loadedQuery = true
            val saved = core.store?.load<ListQuery>(ListQueryKeys.latest)
            if (saved != null && saved != query) {
                // A genuinely DIFFERENT restored query: adopt it so LaunchedEffect(query)
                // refires (this run is stale). Equal values fall through — an equal
                // assignment never refires, and returning here left the tab blank.
                query = saved
                return
            }
        }
        val snapshot = query
        core.scope.launch { core.store?.save(snapshot, ListQueryKeys.latest) }
        // Debounce typing: LaunchedEffect(query) cancels this delay on the
        // next keystroke, so only the settled query actually fetches.
        if (query.search.isNotEmpty()) delay(300)
        // Cache-paint only a COLD list. This runs on every tab return, and
        // the cache holds one page — painting it over a deep in-memory list
        // truncated everything past page 1 and threw the scroll away.
        if (episodes.isEmpty() && isDefaultish && !isDeviceSet) {
            val cached = core.store?.load<List<EpisodeData>>(CacheKey.latest(query.filters))
            if (cached != null) {
                episodes = cached
                loaded = true
                restoreAnchorIfNeeded()
            }
        }
        refresh()
        restoreAnchorIfNeeded()
    }

    /// Cache only canonical views (no search, default order) — one snapshot
    /// per chip-set, same as before the controls bar.
    private val isDefaultish: Boolean
        get() = query.search.isEmpty() &&
            query.orderField == EpisodeOrderField.Published &&
            query.direction == OrderDirection.Desc

    /// OnDevice chip on a non-embedded account: the list IS the local device
    /// set (the server can't express it) — rendered wholesale, no paging.
    private val isDeviceSet: Boolean
        get() = query.filters.contains(EpisodeFilter.OnDevice) && !core.isEmbeddedAccount

    /// Replace with a fresh first page and re-snapshot the cache. On failure
    /// the cached content stays on screen; the error only surfaces when
    /// there's nothing better to show.
    suspend fun refresh() {
        generation += 1
        val mine = generation
        if (isDeviceSet) {
            // Fully local: filter + sort the device set in memory (the
            // remaining chips / search / order still apply — web parity).
            val device = core.models?.device
            val all = (device?.onDevice ?: emptyList()).filter {
                device?.stateOf(it.id) == DeviceDownloads.State.Downloaded
            }
            episodes = query.apply(all, status = { core.overlayStatus(it) })
            hasMore = false
            page = 0
            error = null
            loaded = true
            return
        }
        try {
            var first = core.latestEpisodesRaw(query.queryItems, page = 0, pageSize = 20)
            // A multi-chip facet can't ride the wire — trim the page locally
            // (OR within the facet, same rows the web's local query keeps).
            if (query.needsLocalChipFilter) {
                first = HalogenClient.PageOf(
                    items = first.items.filter {
                        query.matchesChips(it, status = { e -> core.overlayStatus(e) })
                    },
                    hasMore = first.hasMore,
                )
            }
            // A newer query superseded this response — drop it.
            if (mine != generation) return
            // Deep-scroll preserving replace: when the fresh first page matches the
            // loaded prefix, update rows IN PLACE and keep the tail — wholesale replace
            // reset the list to one page per tab return; any real change up top still replaces.
            val freshIds = first.items.map { it.id }
            if (episodes.size > first.items.size &&
                episodes.take(first.items.size).map { it.id } == freshIds
            ) {
                episodes = first.items + episodes.drop(first.items.size)
                // `page` stays where loadMore left it; hasMore from page 0 is
                // still truthful ("more than one page exists").
                hasMore = first.hasMore
            } else {
                episodes = first.items
                hasMore = first.hasMore
                page = 0
            }
            error = null
            if (isDefaultish) {
                // Persist WITHOUT shrinking the snapshot back to one page:
                // fresh page 0 leads, previously cached rows keep the tail —
                // pages the user scrolled through stay renderable offline.
                val ids = first.items.map { it.id }.toSet()
                var snapshot = first.items
                val prior = core.store?.load<List<EpisodeData>>(CacheKey.latest(query.filters))
                if (prior != null) snapshot = snapshot + prior.filter { it.id !in ids }
                core.store?.save(snapshot, CacheKey.latest(query.filters))
            }
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            DeviceLog.warn("latest: refresh failed — ${e::class.simpleName}: ${e.message}")
            if (mine != generation) return
            if (episodes.isEmpty()) error = FriendlyError.message(e)
        }
        loaded = true
    }

    /// Unsubscribe cascade: drop the podcast's rows from the visible list
    /// (the snapshots are pruned by the core's purge).
    fun removePodcastLocally(podcastId: Int) {
        episodes = episodes.filterNot { it.podcast_id == podcastId }
    }

    /// Append the next page (infinite scroll). Id-deduped: rows can shift
    /// between the refresh and append windows.
    suspend fun loadMore() {
        if (!hasMore || isLoadingMore || !loaded || isDeviceSet) return
        // Offline (manual or detected): no paging. Without this the tail
        // sentinel kept firing server fetches while "offline".
        if (core.isOffline) return
        isLoadingMore = true
        loadMoreFailed = false
        // Same generation guard as refresh: a query/filter change mid-fetch
        // must drop this page, not append the OLD facet's rows into the new
        // list (and corrupt page/hasMore for the wrong facet).
        val mine = generation
        try {
            val next = core.latestEpisodesRaw(query.queryItems, page = page + 1, pageSize = 20)
            if (mine != generation) return
            page += 1
            val known = episodes.map { it.id }.toSet()
            var items = next.items.filter { it.id !in known }
            if (query.needsLocalChipFilter) {
                items = items.filter {
                    query.matchesChips(it, status = { e -> core.overlayStatus(e) })
                }
            }
            episodes = episodes + items
            hasMore = next.hasMore
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            DeviceLog.warn("latest: loadMore failed — ${e::class.simpleName}: ${e.message}")
            loadMoreFailed = true
        } finally {
            isLoadingMore = false
        }
    }

    // ── visibility / scroll anchor ──────────────────────────────────────────

    fun rowAppeared(episode: EpisodeData) {
        visible.add(episode.id)
        // Nearing the tail triggers the next page.
        val idx = episodes.indexOfFirst { it.id == episode.id }
        if (idx >= 0 && idx >= episodes.size - 8) {
            core.scope.launch { loadMore() }
        }
    }

    fun rowDisappeared(episode: EpisodeData) {
        visible.remove(episode.id)
    }

    /// Called when the app leaves the foreground (and on tab disappear) —
    /// the anchor survives relaunch, not just tab switches.
    fun persistScrollAnchor() {
        val store = core.store ?: return
        val anchor = topVisibleId ?: return
        core.scope.launch { store.save(anchor, CacheKey.latestScrollAnchor) }
        // Snapshot the whole loaded window (capped), not just page 1: a deep
        // anchor is only restorable when the relaunch cache-paint actually
        // contains its row. `refresh()` then prefix-merges on top.
        if (isDefaultish && !isDeviceSet && episodes.size > 20) {
            val window = episodes.take(200)
            core.scope.launch { store.save(window, CacheKey.latest(query.filters)) }
        }
    }

    private val topVisibleId: Int?
        get() = episodes.firstOrNull { it.id in visible }?.id

    private suspend fun restoreAnchorIfNeeded() {
        if (restoredAnchor || episodes.isEmpty()) return
        val store = core.store ?: return
        restoredAnchor = true
        val anchor = store.load<Int>(CacheKey.latestScrollAnchor) ?: return
        if (episodes.any { it.id == anchor }) pendingScrollTo = anchor
    }
}
