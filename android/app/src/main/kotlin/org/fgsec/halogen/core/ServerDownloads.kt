package org.fgsec.halogen.core

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import org.fgsec.halogen.storage.OutboxOp
import org.fgsec.halogen.wire.DownloadStatus
import org.fgsec.halogen.wire.EpisodeData

/// Live server-side download tracking: poll the per-episode progress endpoint
/// (~1.5s) until the tracker drops the entry, then confirm final status — rows
/// read `progress` (cloud ring) and `completed` without a full list refresh.
class ServerDownloads(
    private val core: HalogenCore,
    private val scope: CoroutineScope,
) {
    /// episode id → 0…1 while the server download runs.
    var progress by mutableStateOf<Map<Int, Double>>(emptyMap())
        private set

    /// Episodes confirmed DOWNLOADED after a tracked run (row-state override).
    var completed by mutableStateOf<Set<Int>>(emptySet())
        private set

    /// Episodes whose tracked run ended NOT downloaded (error surfaced).
    var failed by mutableStateOf<Set<Int>>(emptySet())
        private set

    /// Episodes optimistically removed from the server (the queued
    /// RemoveServerDownload op hasn't drained/refreshed yet) — row snapshots
    /// still say DOWNLOADED, this overlay wins.
    var removed by mutableStateOf<Set<Int>>(emptySet())
        private set

    private val tasks = mutableMapOf<Int, Job>()

    fun progress(episodeId: Int): Double? = progress[episodeId]

    fun isDownloaded(episode: EpisodeData): Boolean {
        if (episode.id in removed) return false
        return episode.id in completed ||
            (episode.download_status == DownloadStatus.Downloaded && episode.id !in failed)
    }

    /// Optimistic local reset paired with an enqueued RemoveServerDownload op
    /// — the row flips immediately (offline too) instead of waiting for the
    /// drain + next refresh.
    fun markRemovedLocally(episodeId: Int) {
        removed = removed + episodeId
        completed = completed - episodeId
        progress = progress - episodeId
    }

    /// Trigger the server download (idempotent server-side) and start polling.
    fun download(episode: EpisodeData) {
        val id = episode.id
        if (tasks[id] != null) return
        progress = progress + (id to 0.0)
        failed = failed - id
        removed = removed - id
        tasks[id] = scope.launch { track(id) }
    }

    /// Watch an already-running server download (rows showing DOWNLOADING).
    fun watch(episodeId: Int) {
        if (tasks[episodeId] != null) return
        progress = progress + (episodeId to (progress[episodeId] ?: 0.0))
        tasks[episodeId] = scope.launch { poll(episodeId) }
    }

    private suspend fun track(id: Int) {
        // Durable trigger (survives offline periods and restarts). While the
        // op is still queued the poll below simply finds no progress; the
        // drain ships it when the server is reachable and a later
        // watch/refresh picks the run back up.
        core.outbox?.enqueue(OutboxOp.Kind.TriggerDownload(id))
        poll(id)
    }

    private suspend fun poll(id: Int) {
        try {
            var notStarted = 0
            loop@ for (i in 0 until 400) {  // ~10 min ceiling at 1.5s steps
                delay(1500)
                val snapshot = orNull { core.downloadProgress(id) }
                if (snapshot != null) {
                    notStarted = 0
                    val percent = snapshot.percent
                    val total = snapshot.total_bytes
                    if (percent != null) {
                        progress = progress + (id to percent.toDouble())
                    } else if (total != null && total > 0uL) {
                        progress = progress +
                            (id to snapshot.bytes_downloaded.toLong().toDouble() / total.toLong().toDouble())
                    }
                    continue
                }
                // No tracker entry: finished, failed, OR not started yet — the
                // episode's status disambiguates. A transport error just retries.
                val fresh = orNull("episode-detail poll") { core.episodeDetail(id) } ?: continue
                when (fresh.download_status) {
                    DownloadStatus.Downloading ->
                        notStarted = 0  // running; the tracker entry will (re)appear
                    DownloadStatus.NotDownloaded -> {
                        notStarted += 1
                        if (notStarted >= STARTUP_GRACE_POLLS) break@loop
                    }
                    else -> break@loop  // downloaded or terminal — confirm below
                }
            }
            progress = progress - id
            val fresh = orNull("final episode-detail") { core.episodeDetail(id) }
            if (fresh != null) {
                if (fresh.download_status == DownloadStatus.Downloaded) {
                    completed = completed + id
                    removed = removed - id
                } else {
                    failed = failed + id
                    DeviceLog.warn("server-download $id: ended ${fresh.download_status.string}")
                }
            } else {
                // Outcome unknown (offline at the final check). NO terminal
                // state here cleared the ring and left watchers spinning
                // forever — mark failed so the row is retryable; the next
                // refresh restores truth.
                failed = failed + id
                DeviceLog.warn("server-download $id: outcome unknown — detail fetch failed")
            }
        } finally {
            tasks.remove(id)
        }
    }

    /// The Swift `try?` shape: any thrown error collapses to null, but
    /// cancellation still propagates. A label makes the collapse loggable.
    private inline fun <T> orNull(label: String? = null, block: () -> T?): T? = try {
        block()
    } catch (e: CancellationException) {
        throw e
    } catch (e: Exception) {
        if (label != null) {
            DeviceLog.warn("server-download: $label failed — ${e::class.simpleName}: ${e.message}")
        }
        null
    }

    companion object {
        /// Consecutive "no tracker entry AND episode still NotDownloaded"
        /// polls tolerated before giving up — covers the startup window where
        /// the TriggerDownload op is still draining through the outbox and the
        /// server hasn't registered the run yet (the first-tap race).
        private const val STARTUP_GRACE_POLLS = 20  // ≈30s at 1.5s steps
    }
}
