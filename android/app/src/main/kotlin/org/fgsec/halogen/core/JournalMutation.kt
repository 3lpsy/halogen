package org.fgsec.halogen.core

import kotlinx.coroutines.launch
import org.fgsec.halogen.wire.EpisodeData
import org.fgsec.halogen.storage.CacheKey
import org.fgsec.halogen.storage.OutboxOp
import org.fgsec.halogen.components.ToastCenter

/** Persist intent before changing cached state; a profile switch cancels the UI update. */
suspend fun HalogenCore.ensureQueued(operation: OutboxOp.Kind): Boolean {
    val queue = outbox
    if (queue == null) {
        ToastCenter.error("Sync storage is unavailable. This change was not saved.")
        return false
    }
    return queue.enqueue(operation) && outbox === queue
}

fun HalogenCore.enqueueMutation(operation: OutboxOp.Kind, episode: EpisodeData? = null, apply: () -> Unit) {
    val queue = outbox
    val snapshotStore = store
    scope.launch {
        if (queue == null) {
            ToastCenter.error("Sync storage is unavailable. This change was not saved.")
        } else {
            try {
                if (episode != null) snapshotStore?.saveDurably(episode, CacheKey.episode(episode.id))
                if (queue.enqueue(operation) && outbox === queue) apply()
            } catch (error: kotlinx.coroutines.CancellationException) { throw error }
            catch (_: Exception) { ToastCenter.error("Couldn't save the episode. Please try again.") }
        }
    }
}

/** Compound actions persist all operations in the same journal transaction. */
suspend fun HalogenCore.ensureQueuedBatch(operations: List<OutboxOp.Kind>): Boolean {
    val queue = outbox
    if (queue == null) {
        ToastCenter.error("Sync storage is unavailable. This change was not saved.")
        return false
    }
    return queue.enqueueBatch(operations) && outbox === queue
}
