package org.fgsec.halogen.features.history

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.async
import kotlinx.coroutines.awaitAll
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import org.fgsec.halogen.components.EpisodeOrderField
import org.fgsec.halogen.components.ListQuery
import org.fgsec.halogen.core.DeviceDownloads
import org.fgsec.halogen.core.DeviceLog
import org.fgsec.halogen.core.FriendlyError
import org.fgsec.halogen.core.HalogenCore
import org.fgsec.halogen.networking.WireJson
import org.fgsec.halogen.storage.CacheKey
import org.fgsec.halogen.wire.EpisodeData
import org.fgsec.halogen.wire.PlaybackData

/// History: episodes ordered by playback recency across the WHOLE set (updated_at
/// desc, episode-id-desc tiebreak — never publish date); bodies resolve into a local
/// pool so search/chips/sorts apply over everything loaded (web query.rs id-list branch).
class HistoryModel(private val core: HalogenCore) {
    private val accountStore = core.store


    /// The displayed rows (pool → chips/search → order).
    var episodes: List<EpisodeData> by mutableStateOf(emptyList())
        private set
    var loaded: Boolean by mutableStateOf(false)
        private set
    var error: String? by mutableStateOf(null)
        private set
    var hasMore: Boolean by mutableStateOf(false)
        private set
    /// Last loadMore failed — the sentinel renders a retry (LoadMoreRow).
    var loadMoreFailed: Boolean by mutableStateOf(false)
        private set

    /// Search + chips + order (chips/search apply locally over the pool).
    /// Persisted across relaunch (web use_list_view_state("history")).
    private var queryState: ListQuery by mutableStateOf(
        ListQuery(orderField = EpisodeOrderField.Recency))
    var query: ListQuery
        get() = queryState
        set(value) {
            if (value == queryState) return
            queryState = value
            val snapshot = value
            core.scope.launch { accountStore?.save(snapshot, "listquery-history") }
            rebuild()
        }
    private var loadedQuery = false

    private var page = 0
    private var loadingMore = false
    private var generation = 0
    /// Resolved episode bodies by id — History's slice of the local pool.
    private val pool = mutableMapOf<Int, EpisodeData>()
    /// Server playback recency (epoch millis) by episode id (from the paged
    /// /playbacks).
    private val recency = mutableMapOf<Int, Long>()

    suspend fun load() {
        error = null
        if (!loadedQuery) {
            loadedQuery = true
            accountStore?.load<ListQuery>("listquery-history")?.let { query = it }
        }
        if (pool.isEmpty()) {
            accountStore?.load<List<EpisodeData>>(CacheKey.history)?.let { cached ->
                for (episode in cached) pool[episode.id] = episode
                rebuild()
                loaded = true
            }
        }
        refresh()
    }

    suspend fun refresh() {
        // Debounce typing (the view's keyed effect cancels the sleep per keystroke).
        if (query.search.isNotEmpty()) delay(300)
        generation += 1
        val mine = generation
        try {
            val first = core.playbacksPage(page = 0)
            if (mine != generation) return
            merge(first.items)
            resolveBodies(first.items.map { it.episode_id })
            if (mine != generation) return
            hasMore = first.hasMore
            page = 0
            error = null
            rebuild()
            persistSnapshot()
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            DeviceLog.warn("history: refresh failed — ${e::class.simpleName}: ${e.message}")
            if (mine != generation) return
            rebuild()
            if (episodes.isEmpty()) error = FriendlyError.message(e)
        }
        loaded = true
    }

    /// Infinite scroll: the next /playbacks page, merged + re-sorted over the
    /// WHOLE pool (appending without a re-sort interleaved page-shaped blocks).
    suspend fun loadMore() {
        if (!hasMore || loadingMore || !loaded) return
        loadingMore = true
        loadMoreFailed = false
        try {
            val next = core.playbacksPage(page = page + 1)
            page += 1
            hasMore = next.hasMore
            merge(next.items)
            resolveBodies(next.items.map { it.episode_id })
            rebuild()
            persistSnapshot()
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            DeviceLog.warn("history: loadMore failed — ${e::class.simpleName}: ${e.message}")
            loadMoreFailed = true
        } finally {
            loadingMore = false
        }
    }

    /// Optimistic entry from a local played-toggle: History gains the episode
    /// immediately (offline included) — the overlay's write timestamp is what
    /// ranks it first (web: the playbacks overlay drives History membership).
    fun noteLocalPlayback(episode: EpisodeData) {
        if (pool[episode.id] != null) {
            rebuild()
            return
        }
        pool[episode.id] = episode
        rebuild()
        persistSnapshot()
    }

    // ── pool plumbing ────────────────────────────────────────────────────

    /// Fold one /playbacks page into the recency map. A row whose episode has
    /// a NEWER local overlay write is skipped — the server can't outrank a
    /// not-yet-drained local toggle (web: merge_history_page's pending-op
    /// guard).
    private fun merge(playbacks: List<PlaybackData>) {
        for (playback in playbacks) {
            val serverAt = epochMillis(playback.updated_at)
            val entry = core.models?.playbacks?.entries?.get(playback.episode_id)
            if (entry != null && entry.updatedAt > serverAt) continue
            val existing = recency[playback.episode_id] ?: Long.MIN_VALUE
            if (serverAt > existing) recency[playback.episode_id] = serverAt
        }
    }

    /// Fetch bodies the pool doesn't hold yet (detail fetch embeds Podcast +
    /// Playback, so rows can name their show and draw progress).
    private suspend fun resolveBodies(ids: List<Int>) {
        val missing = ids.filter { pool[it] == null }
        if (missing.isEmpty()) return
        val fetched = coroutineScope {
            missing.map { id ->
                async {
                    try {
                        core.episodeDetail(id)
                    } catch (e: CancellationException) {
                        throw e
                    } catch (e: Exception) {
                        DeviceLog.warn(
                            "history: episode $id detail failed — ${e::class.simpleName}: ${e.message}")
                        null
                    }
                }
            }.awaitAll()
        }.filterNotNull()
        for (episode in fetched) pool[episode.id] = episode
    }

    /// Recompute the displayed rows: chips + search over the pool, then the
    /// order — recency (updated_at desc, id desc tiebreak; overlay-wins) for
    /// the default query, the user's explicit sort otherwise.
    private fun rebuild() {
        val filterOnly = query.copy(orderField = EpisodeOrderField.Position) // filter without sorting
        var rows = filterOnly.apply(
            pool.values.toList(),
            isOnDevice = { id ->
                core.models?.device?.stateOf(id) == DeviceDownloads.State.Downloaded
            },
            status = { core.overlayStatus(it) },
        )
        if (query.orderField == EpisodeOrderField.Recency) {
            // The dedicated recency field (History's default) — an explicit
            // Published sort is a REAL choice now, not the sentinel.
            val dates = rows.associate { it.id to recencyDate(it) }
            rows = rows.sortedWith(
                compareByDescending<EpisodeData> { dates[it.id] ?: Long.MIN_VALUE }
                    .thenByDescending { it.id })
        } else {
            rows = ListQuery(orderField = query.orderField, direction = query.direction)
                .apply(rows)
        }
        episodes = rows
    }

    /// Freshest known playback moment (epoch millis) for ordering: the local
    /// overlay write outranks the server row when newer (offline listening
    /// ranks first).
    private fun recencyDate(episode: EpisodeData): Long {
        var server = recency[episode.id] ?: Long.MIN_VALUE
        episode.playback?.updated_at?.let { raw ->
            val at = epochMillis(raw)
            if (at > server) server = at
        }
        val entry = core.models?.playbacks?.entries?.get(episode.id)
        if (entry != null && entry.updatedAt > server) return entry.updatedAt
        return server
    }

    /// Persist the POOL (not the filtered view) — chips/search must not
    /// shrink what the next cold start can render. Bodies embed their
    /// playback row, so the reload can re-derive recency order offline.
    private fun persistSnapshot() {
        val snapshot = pool.values.toList()
        core.scope.launch { accountStore?.save(snapshot, CacheKey.history) }
    }

    /// One undecodable wire date must not crash the list — rank it last.
    private fun epochMillis(raw: String): Long =
        runCatching { WireJson.parseInstant(raw).toEpochMilli() }.getOrDefault(Long.MIN_VALUE)
}
