package org.fgsec.halogen.storage

import java.util.UUID
import org.fgsec.halogen.components.ToastCenter
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
import kotlinx.coroutines.yield
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import uniffi.halogen_mobile.SyncQueue
import uniffi.halogen_mobile.SyncDrainReport
import uniffi.halogen_mobile.openSyncQueue
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json
import org.fgsec.halogen.core.DeviceLog
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

/** Native UI adapter over the shared Rust journal and retry engine. */
class Outbox private constructor(
    private val store: LocalStore,
    private val queue: SyncQueue,
    private val perform: suspend (SyncQueue) -> SyncDrainReport,
    private val pull: suspend (SyncQueue) -> String?,
    private val onDeadLetter: suspend (OutboxOp, Throwable) -> Unit,
    initial: List<OutboxOp>,
) {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val mutex = Mutex()
    private val gate = JournalGate.forPath(store.syncDatabasePath)
    private val ops = initial.toMutableList()
    private var draining = false
    @Volatile var suspended = false
        private set

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

    suspend fun clearAll() {
        gate.withLock { mutex.withLock {
            queue.clearAll()
            ops.clear()
            store.saveDurably(ops.toList(), KEY)
        } }
    }

    suspend fun enqueue(kind: OutboxOp.Kind): Boolean = enqueueBatch(listOf(kind))

    suspend fun enqueueBatch(kinds: List<OutboxOp.Kind>): Boolean {
        var journaled = false
        try {
            gate.withLock { mutex.withLock {
                val operations = kinds.map(::OutboxOp)
                queue.importOperations(operations.map { it.sharedOperation() })
                journaled = true
                for (operation in operations) store.project(operation)
                val pending = queue.pendingIds().toSet()
                ops.removeAll { it.id !in pending }
                ops.addAll(operations.filter { it.id in pending })
                // Rebuilt from canonical pending entries if this derived mirror write fails.
                store.save(ops.toList(), KEY)
            } }
        } catch (error: CancellationException) {
            throw error
        } catch (error: Exception) {
            DeviceLog.error("sync persistence failed: $error")
            ToastCenter.error(if (journaled) "The change is queued, but its local cache could not update. Reopen the app to recover it." else "Couldn't save this change. Please try again.")
            return false
        }
        scope.launch { yield(); drain() }
        return true
    }

    suspend fun pendingOperations(): List<OutboxOp> = mutex.withLock { ops.toList() }

    suspend fun setSuspended(value: Boolean) { mutex.withLock { suspended = value } }

    suspend fun drain() {
        mutex.withLock { if (suspended || draining) return; draining = true }
        val failures = mutableListOf<Pair<OutboxOp, Throwable>>()
        try {
            gate.withLock {
                if (suspended) return@withLock
                val submitted = mutex.withLock { ops.toList() }
                queue.importOperations(submitted.map { it.sharedOperation() })
                val report = perform(queue)
                projectAndCheckpoint(store, queue)
                if (!suspended && !report.authPaused) pull(queue)?.let { store.projectSnapshot(it) }
                val rejected = queue.quarantinedIds().toSet()
                val pending = queue.pendingIds().toSet()
                mutex.withLock {
                    for (op in ops.filter { it.id in rejected }) failures.add(op to IllegalStateException(report.lastError ?: "Change rejected by the server"))
                    ops.removeAll { it.id !in pending }
                    store.saveDurably(ops.toList(), KEY)
                }
            }
        } catch (error: CancellationException) { throw error }
        catch (error: Exception) { DeviceLog.warn("sync paused: $error") }
        finally { mutex.withLock { draining = false } }
        for ((operation, error) in failures) onDeadLetter(operation, error)
    }

    companion object {
        private const val KEY = "outbox"
        private val json = Json { ignoreUnknownKeys = true; encodeDefaults = true }

        private suspend fun projectAndCheckpoint(store: LocalStore, queue: SyncQueue) {
            val delivered = queue.deliveredIds().toSet()
            val confirmed = mutableListOf<String>()
            for (operation in restoreSharedQueue(queue.pending())) {
                if (store.project(operation) && operation.id in delivered) confirmed.add(operation.id)
            }
            if (confirmed.isNotEmpty()) {
                queue.confirmCached(confirmed)
                store.forgetJournalMarkers(confirmed.toSet())
            }
        }

        suspend fun create(
            store: LocalStore,
            perform: suspend (SyncQueue) -> SyncDrainReport,
            pull: suspend (SyncQueue) -> String?,
            onDeadLetter: suspend (OutboxOp, Throwable) -> Unit = { _, _ -> },
        ): Outbox {
            val failures = mutableListOf<OutboxOp>()
            val outbox = JournalGate.forPath(store.syncDatabasePath).withLock {
                val original = store.prepareOutboxMigration()
                val ops = original?.let { json.decodeFromString<List<OutboxOp>>(it) } ?: emptyList()
                val queue = withContext(Dispatchers.IO) { openSyncQueue(store.syncDatabasePath) }
                queue.importOperations(ops.map { it.sharedOperation() })
                queue.cachedSnapshot()?.let { store.projectSnapshot(it) }
                val rejected = queue.quarantinedIds().toSet()
                failures.addAll(ops.filter { it.id in rejected })
                projectAndCheckpoint(store, queue)
                val active = queue.pendingIds().toSet()
                val remaining = restoreSharedQueue(queue.pending()).filter { it.id in active }
                store.saveDurably(remaining, KEY)
                Outbox(store, queue, perform, pull, onDeadLetter, remaining)
            }
            for (op in failures) onDeadLetter(op, IllegalStateException("Change rejected by the server"))
            return outbox
        }
    }
}
