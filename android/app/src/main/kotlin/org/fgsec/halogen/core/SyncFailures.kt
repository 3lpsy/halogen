package org.fgsec.halogen.core

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import java.time.Instant
import java.util.UUID
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.MainScope
import kotlinx.coroutines.launch
import kotlinx.serialization.Serializable
import org.fgsec.halogen.components.ToastCenter
import org.fgsec.halogen.networking.WireJson
import org.fgsec.halogen.storage.LocalStore
import org.fgsec.halogen.storage.OutboxOp

/// One dead-lettered outbox op: a change the user made that the server
/// permanently rejected or that exhausted its retry budget.
@Serializable
data class SyncFailure(
    val id: String,
    val summary: String,
    val reason: String,
    /// RFC3339 timestamp (same on-disk shape as the iOS store).
    val at: String,
) {
    val atInstant: Instant
        get() = WireJson.parseInstant(at)
}

/// Persisted record of dropped sync ops, plus the toast at drop time — a
/// discarded change must never be silent (the web toasts on outbox
/// dead-letter; DeviceLog alone is not a user surface). Capped ring;
/// surfaced in Settings.
class SyncFailures(
    private val store: LocalStore,
    private val scope: CoroutineScope = MainScope(),
) {
    var failures: List<SyncFailure> by mutableStateOf(emptyList())
        private set

    init {
        scope.launch { failures = store.load<List<SyncFailure>>(KEY) ?: emptyList() }
    }

    val count: Int
        get() = failures.size

    fun record(op: OutboxOp, error: Throwable) {
        val failure = SyncFailure(
            id = UUID.randomUUID().toString(),
            summary = op.kind.summary,
            reason = FriendlyError.message(error),
            at = WireJson.formatInstant(Instant.now()),
        )
        failures = (failures + failure).takeLast(CAP)
        ToastCenter.error("Couldn't sync \"${failure.summary}\" — ${failure.reason}")
        val snapshot = failures
        scope.launch { store.save(snapshot, KEY) }
    }

    fun clear() {
        failures = emptyList()
        scope.launch { store.save(emptyList<SyncFailure>(), KEY) }
    }

    private companion object {
        const val KEY = "sync-failures"
        const val CAP = 20
    }
}

/// Short human phrase for the sync-failure surfaces.
val OutboxOp.Kind.summary: String
    get() = when (this) {
        is OutboxOp.Kind.SetCursor -> "Save playback position"
        is OutboxOp.Kind.SetPlayed -> if (played) "Mark played" else "Mark unplayed"
        is OutboxOp.Kind.AddToPlaylist -> "Add to playlist"
        is OutboxOp.Kind.RemoveFromPlaylist -> "Remove from playlist"
        is OutboxOp.Kind.MoveInPlaylist -> "Reorder playlist"
        is OutboxOp.Kind.ReorderPlaylist -> "Sort playlist"
        is OutboxOp.Kind.UpdatePlaylist -> "Edit playlist"
        is OutboxOp.Kind.MovePlaylist -> "Reorder playlists"
        is OutboxOp.Kind.Subscribe -> "Subscribe ($feedUrl)"
        is OutboxOp.Kind.Unsubscribe -> "Unsubscribe podcast"
        is OutboxOp.Kind.TriggerDownload -> "Download to server"
        is OutboxOp.Kind.RemoveServerDownload -> "Remove server download"
        is OutboxOp.Kind.UpdatePodcastConfig -> "Update podcast settings"
        is OutboxOp.Kind.RemovePodcastConfig -> "Remove podcast settings"
        is OutboxOp.Kind.SetAutoPlaylists -> "Update auto-playlists"
    }
