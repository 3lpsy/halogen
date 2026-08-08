package org.fgsec.halogen.features.queue

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.launch
import org.fgsec.halogen.components.EpisodeOrderField
import org.fgsec.halogen.components.ListQuery
import org.fgsec.halogen.components.ToastCenter
import org.fgsec.halogen.core.DeviceDownloads
import org.fgsec.halogen.core.DeviceLog
import org.fgsec.halogen.core.FriendlyError
import org.fgsec.halogen.core.HalogenCore
import org.fgsec.halogen.storage.CacheKey
import org.fgsec.halogen.storage.OutboxOp
import org.fgsec.halogen.wire.EpisodeData
import org.fgsec.halogen.wire.OrderDirection
import org.fgsec.halogen.wire.PlaylistData

/// The Queue tab. The queue IS the user's default playlist (`is_default`) — never
/// a separate concept. Local-first: cached rows render before the network answers,
/// and membership mutations apply optimistically then sync via the Outbox.
class QueueModel(private val core: HalogenCore) {

    var queue: PlaylistData? by mutableStateOf(null)
        private set
    var episodes: List<EpisodeData> by mutableStateOf(emptyList())
        private set
    var loaded: Boolean by mutableStateOf(false)
        private set
    var error: String? by mutableStateOf(null)
        private set

    /// Search + facet + order, applied LOCALLY (the queue is a small,
    /// position-ordered list already in memory). Persisted across relaunch
    /// (web use_list_view_state("queue")).
    private val queryState = mutableStateOf(defaultQuery)
    var query: ListQuery
        get() = queryState.value
        set(value) {
            if (value == queryState.value) return
            queryState.value = value
            val store = core.store
            core.scope.launch { store?.save(value, QUERY_KEY) }
        }

    private var loadedQuery = false
    /// Bumped synchronously by every optimistic mutation. A refresh captures
    /// it before its fetch and discards the response if it moved — a mutation
    /// landing mid-fetch must never be overwritten by the stale server list
    /// (web: cache_playlists applies its membership guard at CACHE time).
    private var mutationEpoch = 0

    suspend fun load() {
        error = null
        if (!loadedQuery) {
            loadedQuery = true
            core.store?.load<ListQuery>(QUERY_KEY)?.let { queryState.value = it }
        }
        val store = core.store
        if (store != null) {
            if (queue == null) {
                store.load<PlaylistData>(CacheKey.queueMeta)?.let { queue = it }
            }
            val cachedQueue = queue
            if (cachedQueue != null && episodes.isEmpty()) {
                store.load<List<EpisodeData>>(CacheKey.playlistEpisodes(cachedQueue.id))?.let {
                    episodes = it
                    loaded = true
                }
            }
        }
        refresh()
    }

    suspend fun refresh() {
        // Drain BEFORE pulling (the web's hard-coded order everywhere): the
        // server must reflect optimistic queue membership before we read the
        // playlist back, or this fetch wipes a not-yet-shipped offline add.
        core.outbox?.drain()
        try {
            val fresh = core.defaultPlaylist()
            queue = fresh
            if (fresh != null) {
                core.store?.save(fresh, CacheKey.queueMeta)
                // Membership ops still queued for this playlist (the drain
                // couldn't ship them): the local list is AHEAD of the server —
                // keep it, don't overwrite screen/cache with stale membership
                // (web: cache_playlists' membership-preservation invariant).
                if (core.outbox?.hasPendingOps(fresh.id) == true) {
                    loaded = true
                    return
                }
                val epoch = mutationEpoch
                val served = core.playlistEpisodes(fresh.id)
                // Guard AGAIN at cache time: a mutation (or its just-enqueued
                // op) that landed while the fetch was in flight outranks the
                // stale server membership.
                if (epoch != mutationEpoch) {
                    loaded = true
                    return
                }
                if (core.outbox?.hasPendingOps(fresh.id) == true) {
                    loaded = true
                    return
                }
                episodes = served
                core.store?.save(served, CacheKey.playlistEpisodes(fresh.id))
            } else {
                episodes = emptyList()
                core.store?.remove(CacheKey.queueMeta)
            }
            error = null
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            DeviceLog.warn("queue: refresh failed — ${e::class.simpleName}: ${e.message}")
            if (episodes.isEmpty() && queue == null) error = FriendlyError.message(e)
        }
        loaded = true
    }

    /// "No queue yet" flow: create the default playlist (online-only — the
    /// created id anchors every later offline op). Failure toasts — setting
    /// `error` would replace the whole no-queue screen (and its Create
    /// button) with a full-page error.
    suspend fun createQueue() {
        try {
            core.createPlaylist(name = "Queue", isDefault = true)
            refresh()
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            ToastCenter.error("Couldn't create the queue — ${FriendlyError.message(e)}")
        }
    }

    // ── optimistic membership ops (offline-capable via Outbox) ──────────────

    fun remove(episode: EpisodeData) {
        val queue = queue ?: return
        mutationEpoch += 1
        episodes = episodes.filterNot { it.id == episode.id }
        persistSnapshot()
        core.scope.launch {
            core.outbox?.enqueue(
                OutboxOp.Kind.RemoveFromPlaylist(playlistId = queue.id, episodeId = episode.id))
        }
    }

    /// `toIndex` is the row's FINAL resting index (the reorder helper's
    /// commit semantics — not SwiftUI's insertion offset).
    fun move(fromIndex: Int, toIndex: Int) {
        val queue = queue ?: return
        if (fromIndex !in episodes.indices) return
        mutationEpoch += 1
        val list = episodes.toMutableList()
        val moved = list.removeAt(fromIndex)
        list.add(toIndex.coerceIn(0, list.size), moved)
        episodes = list
        val finalIndex = episodes.indexOfFirst { it.id == moved.id }
        if (finalIndex < 0) return
        persistSnapshot()
        core.scope.launch {
            core.outbox?.enqueue(
                OutboxOp.Kind.MoveInPlaylist(
                    playlistId = queue.id, episodeId = moved.id, to = finalIndex))
        }
    }

    fun moveToTop(episode: EpisodeData) {
        val idx = episodes.indexOfFirst { it.id == episode.id }
        if (idx < 0) return
        move(fromIndex = idx, toIndex = 0)
    }

    /// Called from other tabs' episode rows ("Add to Queue"). Lands at the
    /// FRONT (position 0, newest first) by default — the web's
    /// add_to_queue_front pref, applied optimistically and in the drained op.
    fun add(episode: EpisodeData) {
        val queue = queue ?: run {
            // Cold start before the default playlist ever resolved: the tap
            // must not vanish silently (the button renders regardless).
            ToastCenter.error("Queue isn't loaded yet — reconnect once, then retry.")
            return
        }
        if (episodes.any { it.id == episode.id }) return
        mutationEpoch += 1
        val front = core.models?.prefs?.prefs?.addToQueueFront ?: true
        episodes = if (front) listOf(episode) + episodes else episodes + episode
        persistSnapshot()
        core.scope.launch {
            core.outbox?.enqueue(
                OutboxOp.Kind.AddToPlaylist(
                    playlistId = queue.id, episodeId = episode.id,
                    position = if (front) 0 else null))
        }
    }

    /// The rows the view renders. Reordering is only meaningful over the raw
    /// position order with no search/filter applied.
    val displayed: List<EpisodeData>
        get() = query.apply(
            episodes,
            isOnDevice = { id ->
                core.models?.device?.stateOf(id) == DeviceDownloads.State.Downloaded
            },
            status = { core.overlayStatus(it) },
        )

    val reorderable: Boolean
        get() = query == defaultQuery

    fun contains(episode: EpisodeData): Boolean = episodes.any { it.id == episode.id }

    private fun persistSnapshot() {
        val queue = queue ?: return
        // Keep the playlists pool's episode_ids for the queue row in step
        // (menus and counts read it — see PlaylistsModel.setMembership).
        core.models?.playlists?.setMembership(
            playlistId = queue.id, episodeIds = episodes.map { it.id })
        val snapshot = episodes
        val store = core.store
        core.scope.launch { store?.save(snapshot, CacheKey.playlistEpisodes(queue.id)) }
    }

    private companion object {
        const val QUERY_KEY = "listquery-queue"
        val defaultQuery =
            ListQuery(orderField = EpisodeOrderField.Position, direction = OrderDirection.Asc)
    }
}
