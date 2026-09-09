package org.fgsec.halogen.core

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.launch
import kotlinx.serialization.Serializable
import org.fgsec.halogen.networking.WireJson
import org.fgsec.halogen.storage.CacheKey
import org.fgsec.halogen.storage.OutboxOp
import org.fgsec.halogen.wire.EpisodeData
import org.fgsec.halogen.wire.PlaybackStatus

/// One locally-known playback row (cursor + played flag + when we wrote it,
/// epoch millis).
@Serializable
data class LocalPlayback(
    val cursor: ULong,
    val completed: Boolean,
    val updatedAt: Long,
)

/// Locally-authoritative playback state keyed by episode id: every cursor save /
/// played toggle follows durable journal acknowledgement; readers merge
/// overlay-wins-when-newer, so a stale server page can't regress undrained state.
class PlaybackOverlayModel(
    private val core: HalogenCore,
    private val scope: CoroutineScope,
) {
    private val accountStore = core.store

    var entries by mutableStateOf<Map<Int, LocalPlayback>>(emptyMap())
        private set

    /// Hydrate from disk (boot / account switch) so offline listening
    /// progress survives relaunch.
    suspend fun load() {
        val store = accountStore ?: return
        val queue = core.outbox
        val saved = store.load<Map<Int, LocalPlayback>>(CacheKey.playbacks) ?: emptyMap()
        // Keep newer in-memory entries — a save can race the hydrate.
        val merged = saved.toMutableMap()
        for ((id, mem) in entries) {
            val disk = merged[id]
            if (disk == null || mem.updatedAt >= disk.updatedAt) merged[id] = mem
        }
        if (core.store === store && core.outbox === queue) entries = merged
    }

    // MARK: - mutations (optimistic + outbox, the web's command pattern)

    /// Cursor save: durable intent, then the local playback overlay.
    /// History learns about the episode too — one you merely STARTED must
    /// appear there immediately, offline included.
    fun setCursor(episode: EpisodeData, cursor: ULong) {
        core.enqueueMutation(OutboxOp.Kind.SetCursor(episode.id, cursor)) {
            val now = System.currentTimeMillis()
            val entry = (entries[episode.id] ?: LocalPlayback(0u, false, now)).copy(cursor = cursor, updatedAt = now)
            entries = entries + (episode.id to entry)
            core.models?.history?.noteLocalPlayback(episode)
        }
    }

    /// Played toggle: the cursor resets to 0 in lock-step with the server op,
    /// History gains the episode, and the queued op supersedes pending cursor
    /// saves.
    fun markPlayed(episode: EpisodeData, played: Boolean) {
        core.enqueueMutation(OutboxOp.Kind.SetPlayed(episode.id, played)) {
            entries = entries + (episode.id to LocalPlayback(0u, played, System.currentTimeMillis()))
            if (played) core.models?.history?.noteLocalPlayback(episode)
        }
    }

    /// Drop the local overlay entry for one episode — the per-episode
    /// "Remove local data" purge; the server copy re-syncs on the next pull.
    fun purge(episodeId: Int) {
        if (episodeId !in entries) return
        entries = entries - episodeId
        persist()
    }

    // MARK: - overlay-wins readers

    /// The freshest known resume cursor for a row snapshot (null = never
    /// played anywhere we know of).
    fun cursor(episode: EpisodeData): ULong? {
        val entry = entries[episode.id] ?: return episode.playback?.cursor
        val server = episode.playback
        if (server != null && WireJson.parseInstant(server.updated_at).toEpochMilli() > entry.updatedAt) {
            return server.cursor
        }
        return entry.cursor
    }

    /// The freshest known played facet for a row snapshot — drives menus and
    /// toggles so they never offer the wrong action after an offline toggle.
    fun status(episode: EpisodeData): PlaybackStatus {
        val base = episode.playback_status ?: PlaybackStatus.Unplayed
        val entry = entries[episode.id] ?: return base
        val server = episode.playback
        if (server != null && WireJson.parseInstant(server.updated_at).toEpochMilli() > entry.updatedAt) {
            return base
        }
        if (entry.completed) return PlaybackStatus.Finished
        return if (entry.cursor > 0uL) PlaybackStatus.Played else PlaybackStatus.Unplayed
    }

    /// Persist writes are CHAINED: unordered launches could write an older
    /// snapshot after a newer one (each capture races to the store).
    private var persistChain: Job? = null

    private fun persist() {
        val snapshot = entries
        val store = accountStore
        val previous = persistChain
        persistChain = scope.launch {
            previous?.join()
            store?.save(snapshot, CacheKey.playbacks)
        }
    }
}
