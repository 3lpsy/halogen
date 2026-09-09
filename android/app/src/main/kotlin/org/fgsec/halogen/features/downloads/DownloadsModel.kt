package org.fgsec.halogen.features.downloads

import org.fgsec.halogen.core.ensureQueued
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import org.fgsec.halogen.components.ListQuery
import org.fgsec.halogen.core.DeviceLog
import org.fgsec.halogen.core.FriendlyError
import org.fgsec.halogen.core.HalogenCore
import org.fgsec.halogen.storage.CacheKey
import org.fgsec.halogen.storage.OutboxOp
import org.fgsec.halogen.wire.EpisodeData

/// The Downloads tab: On-device (this device's local copies, fully offline)
/// plus the SERVER's download set (Server / Downloading facets). Embedded
/// accounts have no device tier — the server set IS this device.
class DownloadsModel(private val core: HalogenCore) {
    private val accountStore = core.store


    enum class Facet(val rawValue: String) {
        OnDevice("on_device"),
        Downloaded("downloaded"),
        Downloading("downloading");

        val label: String
            get() = when (this) {
                OnDevice -> "On device"
                Downloaded -> "Server"
                Downloading -> "Downloading"
            }

        /// The wire `filter[download_status]` token (server facets only).
        val token: String get() = rawValue.uppercase()
    }

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
    private var page = 0
    private var loadingMore = false
    var facet: Facet by mutableStateOf(
        if (core.isEmbeddedAccount) Facet.Downloaded else Facet.OnDevice)

    /// Search + order (facet fixed by the segmented control). Persisted
    /// across relaunch (web use_list_view_state("downloads")).
    private var queryState: ListQuery by mutableStateOf(ListQuery())
    var query: ListQuery
        get() = queryState
        set(value) {
            if (value == queryState) return
            queryState = value
            val snapshot = value
            core.scope.launch { accountStore?.save(snapshot, "listquery-downloads") }
        }
    private var loadedQuery = false

    /// Embedded accounts have no device tier — the server set IS this device.
    val availableFacets: List<Facet>
        get() = if (core.isEmbeddedAccount) listOf(Facet.Downloaded, Facet.Downloading)
        else Facet.entries.toList()

    /// Unsubscribe cascade: drop the podcast's rows from the visible list
    /// (the snapshots are pruned by the core's purge).
    fun removePodcastLocally(podcastId: Int) {
        episodes = episodes.filterNot { it.podcast_id == podcastId }
    }

    /// The facet whose rows currently populate `episodes` — a facet switch
    /// must repaint from ITS cache, but a mere query keystroke must not.
    private var paintedFacet: Facet? = null

    suspend fun load() {
        error = null
        if (!loadedQuery) {
            loadedQuery = true
            accountStore?.load<ListQuery>("listquery-downloads")?.let { query = it }
        }
        // Cache-paint only a COLD list or a facet switch (siblings' rule):
        // this refires on every query keystroke, and repainting the full
        // unfiltered snapshot over live search results flashes the whole
        // list per key. `loaded` stays as-is (no empty-state blink).
        val facetChanged = paintedFacet != facet
        if ((episodes.isEmpty() || facetChanged) && query.search.isEmpty()) {
            if (facetChanged) episodes = emptyList()
            accountStore?.load<List<EpisodeData>>(CacheKey.downloads(facet.rawValue))?.let { cached ->
                episodes = cached
                loaded = true
            }
        }
        paintedFacet = facet
        refresh()
    }

    suspend fun refresh() {
        // Debounce typing (the view's keyed effect cancels the sleep per keystroke).
        if (query.search.isNotEmpty()) delay(300)
        if (facet == Facet.OnDevice) {
            // The device set is local — query applies in memory, fully offline.
            val all = core.models?.device?.onDevice ?: emptyList()
            episodes = query.apply(all, status = { core.overlayStatus(it) })
            error = null
            loaded = true
            return
        }
        try {
            val first = core.latestEpisodesRaw(
                extra = listOf("filter[download_status]" to facet.token) + query.queryItems,
                page = 0, pageSize = 20)
            episodes = first.items
            hasMore = first.hasMore
            page = 0
            error = null
            if (query.search.isEmpty()) {
                // Prefix-merge like the other list snapshots: fresh page 0
                // leads, the cached tail survives so scrolled-through rows
                // stay renderable offline.
                val ids = first.items.map { it.id }.toSet()
                var snapshot = first.items
                accountStore?.load<List<EpisodeData>>(CacheKey.downloads(facet.rawValue))
                    ?.let { prior -> snapshot = snapshot + prior.filterNot { it.id in ids } }
                accountStore?.save(snapshot, CacheKey.downloads(facet.rawValue))
            }
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            DeviceLog.warn("downloads: refresh failed — ${e::class.simpleName}: ${e.message}")
            if (episodes.isEmpty()) error = FriendlyError.message(e)
        }
        loaded = true
    }

    suspend fun loadMore() {
        if (facet == Facet.OnDevice || !hasMore || loadingMore || !loaded) return
        loadingMore = true
        loadMoreFailed = false
        try {
            val next = core.latestEpisodesRaw(
                extra = listOf("filter[download_status]" to facet.token) + query.queryItems,
                page = page + 1, pageSize = 20)
            page += 1
            val known = episodes.map { it.id }.toSet()
            episodes = episodes + next.items.filterNot { it.id in known }
            hasMore = next.hasMore
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            DeviceLog.warn("downloads: loadMore failed — ${e::class.simpleName}: ${e.message}")
            loadMoreFailed = true
        } finally {
            loadingMore = false
        }
    }

    /// On-device facet: delete the local copy. Server facets: delete the
    /// server file — durable (web: RemoveServerDownload op), so it queues
    /// offline instead of silently failing.
    suspend fun removeDownload(episode: EpisodeData) {
        if (facet == Facet.OnDevice) {
            episodes = episodes.filterNot { it.id == episode.id }
            core.models?.device?.remove(episode.id)
            return
        }
        if (!core.ensureQueued(OutboxOp.Kind.RemoveServerDownload(episodeId = episode.id))) return
        episodes = episodes.filterNot { it.id == episode.id }
        core.models?.serverDownloads?.markRemovedLocally(episode.id)
    }
}
