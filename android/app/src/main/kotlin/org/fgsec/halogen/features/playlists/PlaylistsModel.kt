package org.fgsec.halogen.features.playlists

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Job
import kotlinx.coroutines.launch
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import org.fgsec.halogen.components.ToastCenter
import org.fgsec.halogen.core.DeviceLog
import org.fgsec.halogen.core.FriendlyError
import org.fgsec.halogen.core.HalogenCore
import org.fgsec.halogen.networking.WireJson
import org.fgsec.halogen.storage.CacheKey
import org.fgsec.halogen.storage.OutboxOp
import org.fgsec.halogen.wire.EpisodeData
import org.fgsec.halogen.wire.OrderDirection
import org.fgsec.halogen.wire.PlaylistData

/// The Playlists tab's sort vocabulary — the web offers Custom (manual
/// position) / Name / Created (crates/ui-views playlists/controls.rs FIELDS).
@Serializable
enum class PlaylistSortField {
    @SerialName("custom") Custom,
    @SerialName("name") Name,
    @SerialName("created") Created;

    val label: String
        get() = when (this) {
            Custom -> "Custom"
            Name -> "Name"
            Created -> "Created"
        }
}

/// Sort + search state for the Playlists tab, persisted across navigation and
/// relaunch (web: `use_list_view_state("playlists", ...)`, default Custom asc).
@Serializable
data class PlaylistListQuery(
    val search: String = "",
    val field: PlaylistSortField = PlaylistSortField.Custom,
    val direction: OrderDirection = OrderDirection.Asc,
)

/// The Playlists tab: the user's playlists (queue included, badged), manual
/// position order. Local-first reads; create/delete are online-only (a
/// created id anchors later offline ops — same tradeoff the web makes for
/// playlist CRUD vs membership ops).
class PlaylistsModel(private val core: HalogenCore) {

    var playlists: List<PlaylistData> by mutableStateOf(emptyList())
        private set
    var loaded: Boolean by mutableStateOf(false)
        private set
    var error: String? by mutableStateOf(null)
        private set
    private var loadedQuery = false

    private val queryState = mutableStateOf(PlaylistListQuery())
    var query: PlaylistListQuery
        get() = queryState.value
        set(value) {
            if (value == queryState.value) return
            queryState.value = value
            val store = core.store
            core.scope.launch { store?.save(value, QUERY_KEY) }
        }

    /// Episodes awaiting a playlist choice — the native stand-in for the
    /// web's add-to-playlist picker page (swipe and bulk actions can't host
    /// a submenu). RootView presents the picker dialog while non-empty.
    var pendingPick: List<EpisodeData> by mutableStateOf(emptyList())

    /// The rows the view renders: name search + Custom/Name/Created sort
    /// (web filter: apply_playlist_search_sort; Custom = `position`).
    val displayed: List<PlaylistData>
        get() {
            var rows = playlists
            val q = query.search.trim().lowercase()
            if (q.isNotEmpty()) rows = rows.filter { it.name.lowercase().contains(q) }
            val asc = query.direction == OrderDirection.Asc
            val comparator: Comparator<PlaylistData> = when (query.field) {
                // Pre-field cached rows decode position=null — sort them last.
                PlaylistSortField.Custom ->
                    compareBy<PlaylistData> { it.position ?: Int.MAX_VALUE }.thenBy { it.id }
                PlaylistSortField.Name ->
                    compareBy<PlaylistData> { it.name.lowercase() }.thenBy { it.id }
                PlaylistSortField.Created ->
                    // Parse like iOS (dates, not raw ISO strings); rank bad dates last.
                    compareBy<PlaylistData> {
                        runCatching { WireJson.parseInstant(it.created_at).toEpochMilli() }
                            .getOrDefault(Long.MIN_VALUE)
                    }.thenBy { it.id }
            }
            return rows.sortedWith(if (asc) comparator else comparator.reversed())
        }

    /// Manual drag-reorder is only meaningful over the unfiltered Custom-asc
    /// view, where a row's display index IS its position — and only while the
    /// visible rows are a contiguous position-prefix, or the dropped index
    /// wouldn't be a valid server position (web page.rs rules).
    val reorderable: Boolean
        get() {
            if (query.field != PlaylistSortField.Custom ||
                query.direction != OrderDirection.Asc ||
                query.search.trim().isNotEmpty()
            ) return false
            return displayed.withIndex().all { (idx, pl) -> pl.position == idx }
        }

    /// Boot hydration (web: the worker's `hydrate_from_store`): fill the pool
    /// from the offline snapshot WITHOUT a network refresh — the row menus'
    /// playlist toggles and by-id route resolution need the pool before the
    /// Playlists tab is ever opened. `loaded` stays false.
    suspend fun seed() {
        if (playlists.isNotEmpty()) return
        val cached = core.store?.load<List<PlaylistData>>(CacheKey.playlists) ?: return
        if (playlists.isEmpty()) playlists = cached
    }

    suspend fun load() {
        error = null
        if (!loadedQuery) {
            loadedQuery = true
            core.store?.load<PlaylistListQuery>(QUERY_KEY)?.let { queryState.value = it }
        }
        if (playlists.isEmpty()) {
            core.store?.load<List<PlaylistData>>(CacheKey.playlists)?.let {
                playlists = it
                loaded = true
            }
        }
        refresh()
    }

    suspend fun refresh() {
        // Drain first, and keep the local (optimistic) order while playlist
        // moves are still queued — server truth would snap a drag back
        // (same guard as PlaylistEpisodesModel.refresh for membership ops).
        core.outbox?.drain()
        if (core.outbox?.hasPendingPlaylistMoves() == true) {
            loaded = true
            return
        }
        try {
            val fresh = core.playlistsList()
            // Membership-preservation at CACHE time (web data_ops.rs
            // cache_playlists): a playlist with queued add/remove/move ops
            // keeps its optimistic episode_ids — the server row predates the
            // ops and would flip menu checkmarks/counts back.
            val outbox = core.outbox
            val merged = fresh.map { playlist ->
                val localIds =
                    if (outbox != null && outbox.hasPendingOps(playlist.id))
                        playlists.firstOrNull { it.id == playlist.id }?.episode_ids
                    else null
                if (localIds != null) playlist.copy(episode_ids = localIds) else playlist
            }
            playlists = merged
            error = null
            core.store?.save(merged, CacheKey.playlists)
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            DeviceLog.warn("playlists: refresh failed — ${e::class.simpleName}: ${e.message}")
            if (playlists.isEmpty()) error = FriendlyError.message(e)
        }
        loaded = true
    }

    suspend fun create(
        name: String, description: String?, isDefault: Boolean,
        deleteServerFile: Boolean, deleteClientFile: Boolean,
    ) {
        core.createPlaylist(
            name = name, description = description, isDefault = isDefault,
            deleteServerFile = deleteServerFile, deleteClientFile = deleteClientFile)
        refresh()
    }

    /// Drag-reorder over the Custom-asc view: reassign positions locally so
    /// the sorted projection holds the new order, then queue the durable
    /// MovePlaylist op (web: optimistic reorder + `commands::move_playlist`).
    /// `toIndex` is the row's FINAL resting index.
    fun move(fromIndex: Int, toIndex: Int) {
        if (!reorderable) return
        val rows = displayed.toMutableList()
        if (fromIndex !in rows.indices) return
        val moved = rows.removeAt(fromIndex)
        rows.add(toIndex.coerceIn(0, rows.size), moved)
        val finalIndex = rows.indexOfFirst { it.id == moved.id }
        if (finalIndex < 0) return
        playlists = rows.mapIndexed { idx, pl -> pl.copy(position = idx) }
        persistSnapshot()
        core.scope.launch {
            core.outbox?.enqueue(
                OutboxOp.Kind.MovePlaylist(playlistId = moved.id, to = finalIndex))
        }
    }

    suspend fun delete(playlist: PlaylistData) {
        playlists = playlists.filterNot { it.id == playlist.id }
        core.store?.save(playlists, CacheKey.playlists)
        // Its episode snapshot would otherwise sit on disk forever.
        core.store?.remove(CacheKey.playlistEpisodes(playlist.id))
        try {
            core.deletePlaylist(playlist.id)
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            // The refresh restores the row — without the toast that read as
            // a UI glitch, not a rejected delete.
            ToastCenter.error("Couldn't delete \"${playlist.name}\" — ${FriendlyError.message(e)}")
            refresh()
        }
    }

    /// Queue a picker request (swipe "Add to playlist" / bulk add). Loads the
    /// playlist list lazily so the dialog has options on a cold start.
    fun requestPick(episodes: List<EpisodeData>) {
        if (episodes.isEmpty()) return
        pendingPick = episodes
        if (playlists.isEmpty()) core.scope.launch { load() }
    }

    /// Deep-link fetch-through cache (web `cache_playlists`): a by-id fetched
    /// playlist joins the in-memory pool AND the offline snapshot, so the
    /// next visit — online or offline — renders instantly.
    fun upsert(playlist: PlaylistData) {
        val idx = playlists.indexOfFirst { it.id == playlist.id }
        playlists =
            if (idx >= 0) playlists.toMutableList().also { it[idx] = playlist }
            else playlists + playlist
        persistSnapshot()
    }

    // ── optimistic mutations (offline-capable via Outbox) ───────────────────

    /// "Add to <playlist>" from row menus / the bulk bar: enqueue the durable
    /// op AND patch the playlist's cached episode snapshot + its episode-id
    /// membership, so the target playlist shows the episode immediately —
    /// offline included (web: add_to_playlist_locally + persist_playlist).
    fun add(episode: EpisodeData, to: PlaylistData) {
        core.scope.launch {
            core.outbox?.enqueue(
                OutboxOp.Kind.AddToPlaylist(
                    playlistId = to.id, episodeId = episode.id, position = null))
        }
        patchMembership(to.id) { ids -> if (episode.id in ids) ids else ids + episode.id }
        patchEpisodeCache(to.id) { cached ->
            if (cached.any { it.id == episode.id }) cached else cached + episode
        }
    }

    /// Membership sync from a playlist's OWN screen: detail/queue remove/move mutate
    /// their episode arrays directly, bypassing add/remove here — overwrite the pool's
    /// episode_ids so row-menu checkmarks and counts agree with the visible list.
    fun setMembership(playlistId: Int, episodeIds: List<Int>) {
        val idx = playlists.indexOfFirst { it.id == playlistId }
        if (idx < 0 || playlists[idx].episode_ids == episodeIds) return
        playlists = playlists.toMutableList().also {
            it[idx] = it[idx].copy(episode_ids = episodeIds)
        }
        persistSnapshot()
    }

    /// "Remove from <playlist>" for membership toggles outside the playlist's
    /// own screen (the row menu's checkmarked list): durable op + the same
    /// membership/episode-cache patching as `add`, mirrored (web:
    /// episode_playlists.rs diffs into remove_from_playlist ops).
    fun remove(episode: EpisodeData, from: PlaylistData) {
        core.scope.launch {
            core.outbox?.enqueue(
                OutboxOp.Kind.RemoveFromPlaylist(playlistId = from.id, episodeId = episode.id))
        }
        patchMembership(from.id) { ids -> ids.filterNot { it == episode.id } }
        patchEpisodeCache(from.id) { cached -> cached.filterNot { it.id == episode.id } }
    }

    /// Serialized read-modify-write of a playlist's episode snapshot — each patch
    /// awaits the previous: the bulk picker calls add() N times, and N unchained
    /// jobs loading the same base array before any saves is a classic lost update.
    private var cachePatchChain: Job? = null

    private fun patchEpisodeCache(
        playlistId: Int, transform: (List<EpisodeData>) -> List<EpisodeData>,
    ) {
        val store = core.store
        val previous = cachePatchChain
        cachePatchChain = core.scope.launch {
            previous?.join()
            val key = CacheKey.playlistEpisodes(playlistId)
            val cached = store?.load<List<EpisodeData>>(key) ?: emptyList()
            store?.save(transform(cached), key)
        }
    }

    /// Offline rename: patch screen + snapshot (the queued UpdatePlaylist op
    /// syncs the server; online renames go direct so forms can show errors).
    fun renameLocally(id: Int, name: String) {
        patch(id) { it.copy(name = name) }
    }

    /// Offline full-edit: apply the web form's field set locally (the queued
    /// UpdatePlaylist op syncs the server on drain).
    fun updateLocally(
        id: Int, name: String, description: String?,
        isDefault: Boolean, deleteServerFile: Boolean, deleteClientFile: Boolean,
    ) {
        patch(id) {
            it.copy(
                name = name, description = description,
                on_remove_delete_file_server = deleteServerFile,
                on_remove_delete_file_client = deleteClientFile,
            )
        }
        if (isDefault) markDefaultLocally(id)
    }

    /// Offline make-queue: flip `is_default` locally so the badge moves (the
    /// server demotes the old default in the same transaction on drain).
    fun markDefaultLocally(id: Int) {
        var changed = false
        val next = playlists.map { pl ->
            if (pl.is_default != (pl.id == id)) {
                changed = true
                pl.copy(is_default = pl.id == id)
            } else pl
        }
        if (!changed) return
        playlists = next
        persistSnapshot()
    }

    private fun patch(id: Int, transform: (PlaylistData) -> PlaylistData) {
        val idx = playlists.indexOfFirst { it.id == id }
        if (idx < 0) return
        playlists = playlists.toMutableList().also { it[idx] = transform(it[idx]) }
        persistSnapshot()
    }

    private fun patchMembership(playlistId: Int, transform: (List<Int>) -> List<Int>) {
        val idx = playlists.indexOfFirst { it.id == playlistId }
        val ids = playlists.getOrNull(idx)?.episode_ids ?: return
        val newIds = transform(ids)
        if (newIds == ids) return
        playlists = playlists.toMutableList().also {
            it[idx] = it[idx].copy(episode_ids = newIds)
        }
        persistSnapshot()
    }

    private fun persistSnapshot() {
        val snapshot = playlists
        val store = core.store
        core.scope.launch { store?.save(snapshot, CacheKey.playlists) }
    }

    private companion object {
        const val QUERY_KEY = "listquery-playlists"
    }
}
