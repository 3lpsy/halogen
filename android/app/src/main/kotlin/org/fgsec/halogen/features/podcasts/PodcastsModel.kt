package org.fgsec.halogen.features.podcasts

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import kotlin.coroutines.cancellation.CancellationException
import kotlinx.coroutines.launch
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import org.fgsec.halogen.core.DeviceLog
import org.fgsec.halogen.core.FriendlyError
import org.fgsec.halogen.core.HalogenCore
import org.fgsec.halogen.networking.WireJson
import org.fgsec.halogen.storage.CacheKey
import org.fgsec.halogen.wire.OrderDirection
import org.fgsec.halogen.wire.PodcastData

/// The Podcasts tab's sort vocabulary — the web offers exactly Title / Added.
@Serializable
enum class PodcastSortField {
    @SerialName("title") Title,
    @SerialName("added") Added;

    val label: String
        get() = if (this == Title) "Title" else "Added"
}

/// Sort + search state for the Podcasts tab, persisted across navigation and
/// relaunch (web: `use_list_view_state("podcasts", ...)`, default Title asc).
@Serializable
data class PodcastListQuery(
    val search: String = "",
    val field: PodcastSortField = PodcastSortField.Title,
    val direction: OrderDirection = OrderDirection.Asc,
)

/// View-state for the Podcasts tab: local-first (cached library renders first, the
/// refresh overwrites screen + snapshot), owned by the models registry so the tab
/// keeps list + scroll. Search + sort apply client-side over the loaded pool.
class PodcastsModel(private val core: HalogenCore) {
    private val accountStore = core.store


    var podcasts by mutableStateOf<List<PodcastData>>(emptyList())
        private set
    var loaded by mutableStateOf(false)
        private set
    var error by mutableStateOf<String?>(null)
        private set
    var hasMore by mutableStateOf(false)
        private set
    /// Last loadMore failed — the sentinel renders a retry (LoadMoreRow).
    var loadMoreFailed by mutableStateOf(false)
        private set
    var isLoadingMore by mutableStateOf(false)
        private set
    private var page = 0
    private var loadedQuery = false

    private var queryState by mutableStateOf(PodcastListQuery())

    /// Every set persists (the Swift didSet) — restore included, harmlessly.
    var query: PodcastListQuery
        get() = queryState
        set(value) {
            if (value == queryState) return
            queryState = value
            core.scope.launch { accountStore?.save(value, QUERY_KEY) }
        }

    /// The rows the view renders: title+author search, then Title/Added sort
    /// (case-insensitive title, id tiebreak — web filter.rs).
    val displayed: List<PodcastData>
        get() {
            var rows = podcasts
            val q = query.search.trim().lowercase()
            if (q.isNotEmpty()) {
                rows = rows.filter {
                    it.title.lowercase().contains(q) ||
                        (it.author?.lowercase()?.contains(q) ?: false)
                }
            }
            val asc = query.direction == OrderDirection.Asc
            val comparator = when (query.field) {
                PodcastSortField.Title ->
                    compareBy<PodcastData> { it.title.lowercase() }.thenBy { it.id }
                PodcastSortField.Added ->
                    // Parse like iOS (dates, not raw ISO strings); rank bad dates last.
                    compareBy<PodcastData> {
                        runCatching { WireJson.parseInstant(it.created_at).toEpochMilli() }
                            .getOrDefault(Long.MIN_VALUE)
                    }.thenBy { it.id }
            }
            return rows.sortedWith(if (asc) comparator else comparator.reversed())
        }

    // ── unsubscribe tombstones ──────────────────────────────────────────────

    /// Web worker.rs `unsubscribed`: locally removed ids whose delete op may still be
    /// queued — pages filter against this set so server truth can't resurrect them
    /// mid-drain. A tombstone clears once no Unsubscribe op remains queued.
    private var tombstones = setOf<Int>()
    private var loadedTombstones = false

    private suspend fun loadTombstonesIfNeeded() {
        if (loadedTombstones) return
        loadedTombstones = true
        accountStore?.load<Set<Int>>(CacheKey.podcastTombstones)?.let {
            tombstones = tombstones + it
        }
    }

    /// Optimistic unsubscribe entry point: tombstone + drop from pool/snapshot.
    suspend fun tombstone(id: Int) {
        loadTombstonesIfNeeded()
        tombstones = tombstones + id
        accountStore?.save(tombstones, CacheKey.podcastTombstones)
        removeLocally(id)
    }

    private suspend fun pruneDrainedTombstones() {
        loadTombstonesIfNeeded()
        val outbox = core.outbox ?: return
        if (tombstones.isEmpty()) return
        val kept = mutableSetOf<Int>()
        for (id in tombstones) {
            if (outbox.hasPendingUnsubscribe(podcastId = id)) kept.add(id)
        }
        if (kept != tombstones) {
            tombstones = kept
            accountStore?.save(kept.toSet(), CacheKey.podcastTombstones)
        }
    }

    /// Boot hydration (web: the worker's `hydrate_from_store`): fill the pool
    /// from the offline snapshot WITHOUT a network refresh, so by-id route
    /// resolution and menus work before the tab is ever opened. `loaded`
    /// stays false — the first tab visit still runs the full load cycle.
    suspend fun seed() {
        loadTombstonesIfNeeded()
        if (podcasts.isNotEmpty()) return
        val cached = accountStore?.load<List<PodcastData>>(CacheKey.podcasts) ?: return
        if (podcasts.isEmpty()) {
            podcasts = cached.filter { it.id !in tombstones }
        }
    }

    suspend fun load() {
        error = null
        if (!loadedQuery) {
            loadedQuery = true
            accountStore?.load<PodcastListQuery>(QUERY_KEY)?.let { query = it }
        }
        val cached = accountStore?.load<List<PodcastData>>(CacheKey.podcasts)
        if (cached != null) {
            loadTombstonesIfNeeded()
            podcasts = cached.filter { it.id !in tombstones }
            loaded = true
        }
        refresh()
    }

    suspend fun refresh() {
        try {
            val first = core.podcasts(page = 0)
            // Tombstone filter at CACHE time: a page fetched while an
            // Unsubscribe op is still queued must not resurrect the podcast.
            pruneDrainedTombstones()
            val items = first.items.filter { it.id !in tombstones }
            podcasts = items
            hasMore = first.hasMore
            page = 0
            error = null
            // Persist WITHOUT shrinking the snapshot back to one page: fresh
            // page 0 leads, previously cached rows keep the tail — everything
            // the user ever scrolled through stays renderable offline (web:
            // CachePodcasts upserts every fetched page into the pool).
            val ids = items.map { it.id }.toSet()
            var snapshot = items
            accountStore?.load<List<PodcastData>>(CacheKey.podcasts)?.let { prior ->
                snapshot = snapshot + prior.filter { it.id !in ids && it.id !in tombstones }
            }
            accountStore?.save(snapshot, CacheKey.podcasts)
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            DeviceLog.warn("podcasts: refresh failed — ${e::class.simpleName}: ${e.message}")
            if (podcasts.isEmpty()) error = FriendlyError.message(e)
        }
        loaded = true
    }

    /// Deep-link fetch-through cache (web `cache_podcasts`): a by-id fetched
    /// podcast joins the in-memory pool AND the offline snapshot, so the
    /// next visit — online or offline — renders instantly.
    fun upsert(podcast: PodcastData) {
        val idx = podcasts.indexOfFirst { it.id == podcast.id }
        podcasts =
            if (idx >= 0) podcasts.toMutableList().also { it[idx] = podcast }
            else podcasts + podcast
        core.scope.launch {
            val cached =
                accountStore?.load<List<PodcastData>>(CacheKey.podcasts)?.toMutableList()
                    ?: mutableListOf()
            val cachedIdx = cached.indexOfFirst { it.id == podcast.id }
            if (cachedIdx >= 0) cached[cachedIdx] = podcast else cached.add(podcast)
            accountStore?.save(cached.toList(), CacheKey.podcasts)
        }
    }

    /// Optimistic unsubscribe: drop the podcast from screen + snapshot (the
    /// durable Unsubscribe outbox op syncs the server). The snapshot is
    /// patched in place so the cached tail beyond the loaded window survives.
    fun removeLocally(id: Int) {
        podcasts = podcasts.filterNot { it.id == id }
        val fallback = podcasts
        core.scope.launch {
            val cached = accountStore?.load<List<PodcastData>>(CacheKey.podcasts) ?: fallback
            accountStore?.save(cached.filterNot { it.id == id }, CacheKey.podcasts)
        }
    }

    /// Infinite scroll: append the next library page and extend the offline
    /// snapshot with it (web: every fetched page is upserted into the store).
    suspend fun loadMore() {
        if (!hasMore || isLoadingMore || !loaded) return
        isLoadingMore = true
        loadMoreFailed = false
        try {
            val next = core.podcasts(page = page + 1)
            page += 1
            val known = podcasts.map { it.id }.toSet()
            podcasts = podcasts + next.items.filter {
                it.id !in known && it.id !in tombstones
            }
            hasMore = next.hasMore
            val snapshot = podcasts
            core.scope.launch { accountStore?.save(snapshot, CacheKey.podcasts) }
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            DeviceLog.warn("podcasts: loadMore failed — ${e::class.simpleName}: ${e.message}")
            loadMoreFailed = true
        } finally {
            isLoadingMore = false
        }
    }

    private companion object {
        const val QUERY_KEY = "listquery-podcasts"
    }
}
