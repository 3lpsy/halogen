package org.fgsec.halogen.storage

import java.util.UUID
import kotlin.math.min
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.SerializationException
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonElement
import org.fgsec.halogen.core.DeviceLog
import org.fgsec.halogen.networking.HalogenClient
import org.fgsec.halogen.wire.OrderDirection
import org.fgsec.halogen.wire.PlaylistReorderField
import org.fgsec.halogen.wire.PodcastConfigUpdateData

/// A queued offline mutation — the native analog of the web outbox op
/// vocabulary; ops serialize (kotlinx polymorphic, SerialName = the iOS case
/// name) and persist across relaunches.
@Serializable
data class OutboxOp(val id: String, val kind: Kind) {
    constructor(kind: Kind) : this(UUID.randomUUID().toString(), kind)

    @Serializable
    sealed interface Kind {
        // Playback
        @Serializable
        @SerialName("setCursor")
        data class SetCursor(val episodeId: Int, val cursor: ULong) : Kind

        @Serializable
        @SerialName("setPlayed")
        data class SetPlayed(val episodeId: Int, val played: Boolean) : Kind

        // Playlist membership
        @Serializable
        @SerialName("addToPlaylist")
        data class AddToPlaylist(
            val playlistId: Int, val episodeId: Int, val position: Int? = null
        ) : Kind

        @Serializable
        @SerialName("removeFromPlaylist")
        data class RemoveFromPlaylist(val playlistId: Int, val episodeId: Int) : Kind

        @Serializable
        @SerialName("moveInPlaylist")
        data class MoveInPlaylist(val playlistId: Int, val episodeId: Int, val to: Int) : Kind

        // Playlist structure. `updatePlaylist` is queued only when offline —
        // online edits go direct so forms can show server errors (web rule).
        // Later optional fields default to null for ops persisted before they
        // existed (same back-compat rule as the web's serde defaults).
        @Serializable
        @SerialName("reorderPlaylist")
        data class ReorderPlaylist(
            val playlistId: Int, val field: PlaylistReorderField, val direction: OrderDirection
        ) : Kind

        @Serializable
        @SerialName("updatePlaylist")
        data class UpdatePlaylist(
            val playlistId: Int,
            val name: String? = null,
            val isDefault: Boolean? = null,
            val description: String? = null,
            val deleteServerFile: Boolean? = null,
            val deleteClientFile: Boolean? = null,
        ) : Kind

        @Serializable
        @SerialName("movePlaylist")
        data class MovePlaylist(val playlistId: Int, val to: Int) : Kind

        // Library
        @Serializable
        @SerialName("subscribe")
        data class Subscribe(
            val feedUrl: String, val title: String? = null, val description: String? = null
        ) : Kind

        @Serializable
        @SerialName("unsubscribe")
        data class Unsubscribe(val podcastId: Int) : Kind

        // Server downloads
        @Serializable
        @SerialName("triggerDownload")
        data class TriggerDownload(val episodeId: Int) : Kind

        @Serializable
        @SerialName("removeServerDownload")
        data class RemoveServerDownload(val episodeId: Int) : Kind

        // Podcast config (update/remove target existing ids — safe to drain
        // later; create stays online-only, it needs a real id back).
        @Serializable
        @SerialName("updatePodcastConfig")
        data class UpdatePodcastConfig(val configId: Int, val data: PodcastConfigUpdateData) : Kind

        @Serializable
        @SerialName("removePodcastConfig")
        data class RemovePodcastConfig(val podcastId: Int) : Kind

        // `addToStart` defaults to null for pre-field persisted ops.
        @Serializable
        @SerialName("setAutoPlaylists")
        data class SetAutoPlaylists(
            val podcastId: Int, val playlistIds: List<Int>, val addToStart: Boolean? = null
        ) : Kind
    }
}

/// Persisted FIFO of offline mutations: models apply OPTIMISTICALLY, enqueue, the
/// drain syncs when reachable. Failure taxonomy (web drain): permanent (4xx)
/// dead-letters immediately; countable (5xx/empty) retries with backoff, dropped
/// after `DEAD_LETTER_ATTEMPTS`; transient (transport, 401/408/429) retries forever.
class Outbox private constructor(
    private val store: LocalStore,
    /// Executes one op against the API. Injected by the core (owns the client).
    private val perform: suspend (OutboxOp) -> Unit,
    /// Fired when an op is dead-lettered — the user must hear about a
    /// discarded change (toast + sync-failures record).
    private val onDeadLetter: suspend (OutboxOp, Throwable) -> Unit,
    initial: List<OutboxOp>,
) {
    /// Retry bookkeeping for a failing FIFO head. In-memory only — a relaunch
    /// grants a fresh budget (same as the web).
    private data class HeadRetry(val opId: String, val failures: Int, val skipDrains: Int)

    private val mutex = Mutex()
    private val ops: MutableList<OutboxOp> = initial.toMutableList()

    /// The core flips this with the manual-offline toggle: ops queue but
    /// never drain while suspended.
    var suspended = false
        private set

    private var draining = false
    private var headRetry: HeadRetry? = null

    suspend fun pendingCount(): Int = mutex.withLock { ops.size }

    /// Whether any queued op still targets `playlistId`'s membership/order —
    /// refresh guards keep the local list authoritative until these drain.
    suspend fun hasPendingOps(playlistId: Int): Boolean = mutex.withLock {
        ops.any {
            when (val kind = it.kind) {
                is OutboxOp.Kind.AddToPlaylist -> kind.playlistId == playlistId
                is OutboxOp.Kind.RemoveFromPlaylist -> kind.playlistId == playlistId
                is OutboxOp.Kind.MoveInPlaylist -> kind.playlistId == playlistId
                is OutboxOp.Kind.ReorderPlaylist -> kind.playlistId == playlistId
                else -> false
            }
        }
    }

    /// Whether an Unsubscribe for `podcastId` is still queued — the podcasts
    /// tombstone set keeps its entries until their op drains.
    suspend fun hasPendingUnsubscribe(podcastId: Int): Boolean = mutex.withLock {
        ops.any { (it.kind as? OutboxOp.Kind.Unsubscribe)?.podcastId == podcastId }
    }

    /// Whether any queued op still reorders the playlists themselves — the
    /// playlists-list refresh keeps the local order until the moves drain.
    suspend fun hasPendingPlaylistMoves(): Boolean = mutex.withLock {
        ops.any { it.kind is OutboxOp.Kind.MovePlaylist }
    }

    /// Discard every queued op (the purge screen). Irreversible.
    suspend fun clearAll() {
        mutex.withLock {
            ops.clear()
            headRetry = null
            persist()
        }
    }

    suspend fun enqueue(kind: OutboxOp.Kind) {
        mutex.withLock {
            // Cursor writes coalesce: a fresh save supersedes any queued one
            // for the same episode; a played-toggle writes the cursor too (0),
            // so it supersedes queued cursor saves the same way.
            val coalesceEpisode = when (kind) {
                is OutboxOp.Kind.SetCursor -> kind.episodeId
                is OutboxOp.Kind.SetPlayed -> kind.episodeId
                else -> null
            }
            if (coalesceEpisode != null) {
                ops.removeAll { (it.kind as? OutboxOp.Kind.SetCursor)?.episodeId == coalesceEpisode }
            }
            ops.add(OutboxOp(kind))
            persist()
        }
        drain()
    }

    suspend fun setSuspended(value: Boolean) {
        mutex.withLock { suspended = value }
    }

    /// Pump the queue FIFO — see the class doc for the failure taxonomy.
    /// Draining stops at the head's first transient/backing-off failure so
    /// dependent ops can never replay out of order.
    suspend fun drain() {
        mutex.withLock {
            if (suspended || draining) return
            draining = true
        }
        try {
            drainLoop()
        } finally {
            mutex.withLock { draining = false }
        }
    }

    private suspend fun drainLoop() {
        // A head op backing off after countable failures skips whole drains
        // per its budget. A different head means the old entry is stale —
        // drop it for a fresh budget.
        mutex.withLock {
            val head = ops.firstOrNull()
            if (head != null) {
                val retry = headRetry
                if (retry != null && retry.opId == head.id) {
                    if (retry.skipDrains > 0) {
                        headRetry = retry.copy(skipDrains = retry.skipDrains - 1)
                        return
                    }
                } else {
                    headRetry = null
                }
            }
        }

        while (true) {
            val op = mutex.withLock { ops.firstOrNull() } ?: return
            try {
                perform(op)
                mutex.withLock {
                    if (headRetry?.opId == op.id) headRetry = null
                    removeHead(op)
                    persist()
                }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                when (classify(e)) {
                    FailureClass.PERMANENT -> {
                        // The server will never accept this op — dead-letter it
                        // so it stops blocking the head of the FIFO every drain.
                        DeviceLog.warn("outbox: dropped op after API rejection: $e")
                        mutex.withLock {
                            if (headRetry?.opId == op.id) headRetry = null
                            removeHead(op)
                            persist()
                        }
                        onDeadLetter(op, e)
                    }
                    FailureClass.COUNTABLE -> {
                        // 5xx/empty could equally be a transient outage or a
                        // deterministic server bug on this payload — retry,
                        // but against a budget.
                        val attempt = mutex.withLock {
                            (headRetry?.takeIf { it.opId == op.id }?.failures ?: 0) + 1
                        }
                        if (attempt >= DEAD_LETTER_ATTEMPTS) {
                            DeviceLog.warn(
                                "outbox: op failed every budgeted retry ($attempt); dropping: $e"
                            )
                            mutex.withLock {
                                headRetry = null
                                removeHead(op)
                                persist()
                            }
                            onDeadLetter(op, e)
                        } else {
                            DeviceLog.warn(
                                "outbox: op failed (attempt $attempt); retrying with backoff: $e"
                            )
                            mutex.withLock {
                                headRetry = HeadRetry(op.id, attempt, drainSkips(attempt))
                            }
                            return
                        }
                    }
                    FailureClass.TRANSIENT -> {
                        val pending = mutex.withLock { ops.size }
                        DeviceLog.info("outbox: drain paused (retryable $e), $pending pending")
                        return
                    }
                }
            }
        }
    }

    /// Remove `op` if it is still the head (a concurrent clearAll may have
    /// emptied the queue while `perform` was in flight).
    private fun removeHead(op: OutboxOp) {
        if (ops.firstOrNull()?.id == op.id) ops.removeAt(0)
    }

    private enum class FailureClass { PERMANENT, COUNTABLE, TRANSIENT }

    /// The web's permanent/countable/transient taxonomy over the client's
    /// error shape.
    private fun classify(error: Throwable): FailureClass = when (error) {
        // Validation: the server rejected the payload — permanent.
        is HalogenClient.ClientError.Api -> FailureClass.PERMANENT
        is HalogenClient.ClientError.Http -> when {
            // Token may refresh / explicit throttle-and-retry.
            error.code == 401 || error.code == 408 || error.code == 429 ->
                FailureClass.TRANSIENT
            error.code in 400..499 -> FailureClass.PERMANENT
            error.code >= 500 -> FailureClass.COUNTABLE
            else -> FailureClass.TRANSIENT
        }
        is HalogenClient.ClientError.EmptyData -> FailureClass.COUNTABLE
        // Never counts against an op — waits for reconnect/re-auth.
        is HalogenClient.ClientError.Offline -> FailureClass.TRANSIENT
        is HalogenClient.ClientError.SignedOut -> FailureClass.TRANSIENT
        // Version skew: the op executed but its response no longer decodes as
        // this build expects. Deterministic — burn the countable budget
        // instead of wedging the head as "offline" forever.
        is SerializationException -> FailureClass.COUNTABLE
        // IOException and friends: offline — keep the op, wait for reconnect.
        else -> FailureClass.TRANSIENT
    }

    private suspend fun persist() {
        store.save(ops.toList(), KEY)
    }

    companion object {
        private const val KEY = "outbox"

        /// Consecutive countable failures of the same head op before it is
        /// dead-lettered (web: OUTBOX_DEAD_LETTER_ATTEMPTS).
        private const val DEAD_LETTER_ATTEMPTS = 10

        private val json = Json { ignoreUnknownKeys = true; encodeDefaults = true }

        /// Drains to skip before re-attempting a head op with `failures`
        /// consecutive countable failures: 1, 3, 7, then 15 (cap) — the web's
        /// `drain_skips` backoff curve.
        private fun drainSkips(failures: Int): Int = (1 shl min(failures, 4)) - 1

        suspend fun create(
            store: LocalStore,
            perform: suspend (OutboxOp) -> Unit,
            onDeadLetter: suspend (OutboxOp, Throwable) -> Unit = { _, _ -> },
        ): Outbox {
            // Per-op lossy decode: ONE op persisted by a different build
            // (unknown kind / changed payload) must not wipe the whole queue —
            // an all-or-nothing decode would drop every survivor on the next
            // persist.
            val boxed = store.load<List<JsonElement>>(KEY) ?: emptyList()
            val ops = boxed.mapNotNull { element ->
                try {
                    json.decodeFromJsonElement(OutboxOp.serializer(), element)
                } catch (e: Exception) {
                    DeviceLog.warn("outbox: op decode failed — ${e::class.simpleName}: ${e.message}")
                    null
                }
            }
            if (ops.size < boxed.size) {
                DeviceLog.warn(
                    "outbox: dropped ${boxed.size - ops.size} undecodable persisted op(s); kept ${ops.size}"
                )
            }
            return Outbox(store, perform, onDeadLetter, ops)
        }
    }
}
