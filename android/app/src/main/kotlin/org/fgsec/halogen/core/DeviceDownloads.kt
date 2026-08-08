package org.fgsec.halogen.core

import android.content.Context
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import java.io.File
import java.io.FileOutputStream
import java.io.IOException
import java.util.UUID
import java.util.concurrent.TimeUnit
import kotlin.coroutines.cancellation.CancellationException
import kotlin.math.max
import kotlin.math.min
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.Job
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.delay
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json
import okhttp3.HttpUrl
import okhttp3.OkHttpClient
import okhttp3.Request
import org.fgsec.halogen.networking.Http
import org.fgsec.halogen.networking.await
import org.fgsec.halogen.storage.CacheKey
import org.fgsec.halogen.storage.OutboxOp
import org.fgsec.halogen.wire.DownloadStatus
import org.fgsec.halogen.wire.EpisodeData

// Device downloads (web crates/ui-svc-sync download.rs): phase 1 waits for the
// SERVER copy (durable TriggerDownload op + polling — never the RSS origin);
// phase 2 pulls Range chunks into a `.partial` whose length IS the resume point
// (`.partial.meta` sidecar keeps the Content-Type); only verified-complete files commit.
@OptIn(ExperimentalCoroutinesApi::class)
class DeviceDownloads(
    private val core: HalogenCore,
    context: Context,
    namespace: String,
    private val scope: CoroutineScope,
) {
    sealed interface State {
        data object None : State

        // Phase 1 for an episode the server doesn't hold yet: the server
        // download was triggered and is being awaited (rows show the cloud ring).
        data object WaitingServer : State

        data class Downloading(val progress: Double) : State

        data class Paused(val progress: Double) : State

        // Terminal failure — surfaced instead of masquerading as a pause. Tap retries.
        data class Failed(val message: String) : State

        data object Downloaded : State
    }

    data class Stats(val count: Int, val bytes: Long)

    companion object {
        // Web parity constants (crates/ui-svc-sync download.rs).
        // Server-copy wait budget: tries × interval ≈ 2 minutes.
        private const val SERVER_POLL_TRIES = 40
        private const val SERVER_POLL_INTERVAL_MS = 3_000L

        // Consecutive transport-error polls that mean "offline", not "slow server".
        private const val SERVER_POLL_OFFLINE_GIVEUP = 5

        // Per-chunk attempts before the whole download gives up.
        private const val CHUNK_RETRIES = 5
        private const val BACKOFF_INITIAL_MS = 500L
        private const val BACKOFF_MAX_MS = 8_000L

        // Bytes between flush/progress ticks in the streaming (no-chunking) path.
        private const val STREAM_FLUSH_BYTES = 4 * 1024 * 1024

        // Per-request idle timeouts matching the iOS URLRequest intervals.
        private val chunkClient: OkHttpClient by lazy {
            Http.client.newBuilder().readTimeout(30, TimeUnit.SECONDS).build()
        }
        private val streamClient: OkHttpClient by lazy {
            Http.client.newBuilder().readTimeout(60, TimeUnit.SECONDS).build()
        }

        private val metaJson = Json { ignoreUnknownKeys = true; encodeDefaults = true }

        // File extension for a downloaded content type (web ext_for) — playback
        // resolves local copies by extension. Unknown types fall back to mp3.
        private fun ext(contentType: String?): String {
            val mime = contentType?.split(";")?.firstOrNull()?.trim()?.lowercase()
            return when (mime) {
                "audio/mpeg", "audio/mp3" -> "mp3"
                "audio/mp4", "audio/x-m4a", "audio/aac" -> "m4a"
                "audio/ogg" -> "ogg"
                "audio/opus" -> "opus"
                "audio/wav", "audio/x-wav" -> "wav"
                "audio/flac" -> "flac"
                else -> "mp3"
            }
        }

        // True for `<id>.partial` / `<id>.partial.meta` — staging artifacts, never audio.
        private fun isPartialArtifact(name: String): Boolean =
            name.endsWith(".partial") || name.endsWith(".partial.meta")

        // "bytes 0-1023/4096" → (0, 4096); "bytes */4096" → (null, 4096).
        private fun parseContentRange(value: String?): Pair<Long?, Long?> {
            if (value == null) return null to null
            val spec = value.replace("bytes", "").trim()
            val parts = spec.split("/", limit = 2)
            val total = if (parts.size == 2) parts[1].trim().toLongOrNull() else null
            val start = parts.firstOrNull()?.split("-")?.firstOrNull()?.trim()?.toLongOrNull()
            return start to total
        }
    }

    // Episode id → live state. Finished episodes are also rediscovered from
    // disk after relaunch (scan in `load`).
    var states: Map<Int, State> by mutableStateOf(emptyMap())
        private set

    // Episodes on device, newest-added first (the Downloads page's facet).
    var onDevice: List<EpisodeData> by mutableStateOf(emptyList())
        private set

    private val tasks = mutableMapOf<Int, Job>()

    // Identity of the CURRENT run per episode: a cancelled run's cleanup must
    // not clobber the handle of a restarted download.
    private val runTokens = mutableMapOf<Int, UUID>()

    private val dir: File =
        File(File(File(context.filesDir, "halogen-client"), namespace), "audio")
            .apply { mkdirs() }

    // Rebuild state from disk (finished files + partials) and the cached list.
    suspend fun load() {
        core.store?.load<List<EpisodeData>>(CacheKey.deviceDownloads)?.let { onDevice = it }
        val seeded = withContext(Dispatchers.IO) {
            val entries = mutableMapOf<Int, State>()
            for (file in dir.list().orEmpty()) {
                val id = file.substringBefore(".").toIntOrNull() ?: continue
                if (file.endsWith(".partial")) {
                    // Seed the resume progress from the staged offset + total
                    // (web: resume_partial_downloads' percent seeding).
                    val staged = partialLength(id)
                    val total = loadMeta(id)?.total ?: 0L
                    val progress =
                        if (total > 0) min(staged.toDouble() / total.toDouble(), 1.0) else 0.0
                    entries.putIfAbsent(id, State.Paused(progress))
                } else if (!isPartialArtifact(file)) {
                    entries[id] = State.Downloaded
                }
            }
            entries
        }
        // Finished files always win; a partial only seeds when nothing is live.
        val merged = states.toMutableMap()
        for ((id, state) in seeded) {
            if (state is State.Downloaded) merged[id] = state else merged.putIfAbsent(id, state)
        }
        states = merged
    }

    fun stateOf(episodeId: Int): State = states[episodeId] ?: State.None

    // The local file to play from, when fully downloaded (any extension —
    // commit names the file from the download's content type).
    fun localFile(episodeId: Int): File? = committedFile(episodeId)

    // (file count, total bytes) for the purge screen — finals + partials.
    val stats: Stats
        get() {
            val files = dir.list().orEmpty()
            var bytes = 0L
            for (file in files) bytes += File(dir, file).length()
            return Stats(files.size, bytes)
        }

    // Delete every local copy + partial (the purge screen).
    fun removeAll() {
        for (id in tasks.keys.toList()) pause(id)
        for (file in dir.list().orEmpty()) File(dir, file).delete()
        states = emptyMap()
        onDevice = emptyList()
        persistList()
    }

    // ── control ──────────────────────────────────────────────────────────

    // Boot auto-resume (web resume_partial_downloads): staged partials continue
    // without a manual tap when online; failures stay parked.
    fun resumePartials() {
        if (core.isOffline) return
        for (episode in onDevice) {
            if (stateOf(episode.id) is State.Paused) download(episode)
        }
    }

    fun download(episode: EpisodeData) {
        val id = episode.id
        when (stateOf(id)) {
            is State.Downloaded, is State.Downloading, is State.WaitingServer -> return
            else -> {}
        }
        // Fail fast on known-offline: the device pulls the SERVER's copy, so
        // there's nothing to do. Unknown status deliberately proceeds — local-first.
        if (core.isOffline) {
            putState(id, State.Failed("You're offline — can't download to this device right now."))
            return
        }
        putState(id, State.Downloading(currentProgress(id)))
        rememberOnDevice(episode)
        val token = UUID.randomUUID()
        runTokens[id] = token
        tasks[id] = scope.launch { run(episode, token) }
    }

    fun pause(episodeId: Int) {
        tasks[episodeId]?.cancel()
        tasks.remove(episodeId)
        when (val current = stateOf(episodeId)) {
            is State.Downloading -> putState(episodeId, State.Paused(current.progress))
            // Nothing partial to keep — the server-side fetch (if started) keeps
            // running; dropping the entry reads back as None.
            is State.WaitingServer -> putState(episodeId, null)
            else -> {}
        }
    }

    fun remove(episodeId: Int) {
        pause(episodeId)
        committedFile(episodeId)?.delete()
        discardPartial(episodeId)
        putState(episodeId, null)
        onDevice = onDevice.filter { it.id != episodeId }
        persistList()
    }

    // ── the download task ────────────────────────────────────────────────

    private suspend fun run(episode: EpisodeData, token: UUID) {
        val id = episode.id
        try {
            // Phase 1: make sure the server holds its copy — the audio endpoint
            // 404s otherwise (server audio.rs serves only Downloaded rows).
            val serverReady =
                core.models?.serverDownloads?.isDownloaded(episode)
                    ?: (episode.download_status == DownloadStatus.Downloaded)
            if (!serverReady) {
                putState(id, State.WaitingServer)
                if (!waitForServerCopy(id)) return
                putState(id, State.Downloading(currentProgress(id)))
            }
            currentCoroutineContext().ensureActive()

            // Phase 2: pull the bytes. On failure the durable partial is KEPT
            // (invisible to localFile) for the next manual retry to resume.
            val chunkKiB =
                core.models?.prefs?.prefs?.downloadChunkKiB ?: ClientPrefs.default.downloadChunkKiB
            val parallelism = max(1, core.models?.prefs?.prefs?.downloadParallelism ?: 1)
            if (chunkKiB > 0) {
                downloadChunked(id, chunkLen = max(64, chunkKiB).toLong() * 1024, parallelism)
            } else {
                downloadStreaming(id)
            }
            commit(id)
        } catch (_: CancellationException) {
            return
        } catch (e: Exception) {
            // A cancel that surfaced as an aborted-socket IOException is still a
            // cancel — never overwrite the pause the user just made.
            if (!currentCoroutineContext().isActive) return
            val message = (e as? DownloadError)?.messageText ?: (e.message ?: "download failed")
            DeviceLog.warn("device-download $id: $message")
            putState(id, State.Failed(message))
        } finally {
            // Only THIS run's registration — a restart has its own token.
            if (runTokens[id] == token) {
                tasks.remove(id)
                runTokens.remove(id)
            }
        }
    }

    // Web download.rs phase 1: durable TriggerDownload op, then poll the episode
    // until the server copy is Downloaded — bailing early on manual offline, a
    // terminal server status, or a run of transport errors (unreachable server,
    // not a slow one). Returns false after setting the failure state.
    private suspend fun waitForServerCopy(id: Int): Boolean {
        // Durable + idempotent server-side; survives offline and restarts.
        core.outbox?.enqueue(OutboxOp.Kind.TriggerDownload(episodeId = id))
        // Track the server run too so the waiting row's cloud ring shows live progress.
        core.models?.serverDownloads?.watch(id)
        var consecutiveErrors = 0
        repeat(SERVER_POLL_TRIES) {
            if (!currentCoroutineContext().isActive) return false
            if (core.connection.manualOffline) {
                putState(id, State.Failed("Cancelled — you went offline."))
                return false
            }
            try {
                val fresh = core.episodeDetail(id)
                consecutiveErrors = 0
                when (fresh.download_status) {
                    DownloadStatus.Downloaded -> return true
                    DownloadStatus.DownloadError, DownloadStatus.DownloadUnauthorized,
                    DownloadStatus.DownloadRemoteNotFound, DownloadStatus.DownloadBroken -> {
                        // Terminal: surface the error instead of polling out the budget.
                        DeviceLog.warn(
                            "device-download $id: server fetch ended ${fresh.download_status.string}"
                        )
                        putState(id, State.Failed("The server couldn't fetch the episode."))
                        return false
                    }
                    else -> {}
                }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                DeviceLog.warn(
                    "device-download $id: server poll failed — ${e::class.simpleName}: ${e.message}")
                consecutiveErrors += 1
                if (consecutiveErrors >= SERVER_POLL_OFFLINE_GIVEUP) {
                    putState(
                        id,
                        State.Failed(
                            "You appear to be offline — can't reach the server to download."))
                    return false
                }
            }
            delay(SERVER_POLL_INTERVAL_MS)
        }
        putState(
            id,
            State.Failed(
                if (consecutiveErrors > 0)
                    "You appear to be offline — can't reach the server to download."
                else "Timed out waiting for the server download."))
        return false
    }

    // ── chunked strategy (web download_audio_chunked) ────────────────────

    // One offset-mismatch (a proxy that ignored the Range) discards the partial
    // and re-runs from scratch; a second mismatch means the peer never honors
    // ranges — fall back to streaming, which handles a 200-from-byte-0
    // correctly. Never commits stitched bytes.
    private suspend fun downloadChunked(id: Int, chunkLen: Long, parallelism: Int) {
        try {
            chunkedAttempt(id, chunkLen, parallelism)
        } catch (e: DownloadError.OffsetMismatch) {
            DeviceLog.warn(
                "device-download $id: chunk served from ${e.servedFrom}, asked ${e.requested}; restarting from scratch"
            )
            discardPartial(id)
            try {
                chunkedAttempt(id, chunkLen, parallelism)
            } catch (_: DownloadError.OffsetMismatch) {
                DeviceLog.warn(
                    "device-download $id: server never honors byte ranges; falling back to streaming"
                )
                discardPartial(id)
                downloadStreaming(id)
            }
        }
    }

    // One pass of the chunked download: resume verification → Phase A (first
    // chunk alone) → Phase B′ (unknown-total continuation) → Phase B (up to
    // `parallelism` chunks in flight, written strictly in byte order) → the
    // final completeness check. Mirrors download_audio_chunked_attempt.
    private suspend fun chunkedAttempt(id: Int, chunkLen: Long, parallelism: Int) {
        val base = core.audioUrl(episodeId = id) ?: throw DownloadError.Failed("signed out")
        val token = core.apiToken

        var handle = openPartialForAppend(id)
        try {
            var downloaded = partialLength(id)
            var contentType = if (downloaded > 0) loadMeta(id)?.contentType else null
            var total: Long? = if (downloaded > 0) loadMeta(id)?.total else null
            // Validate the staged partial against the server copy before
            // trusting the bytes already on disk.
            var verifyingResume = downloaded > 0
            if (verifyingResume) {
                DeviceLog.info("device-download $id: resuming at $downloaded bytes")
            }

            // Discard the partial and start over as a fresh download.
            fun restartFromScratch() {
                runCatching { handle.close() }
                discardPartial(id)
                handle = openPartialForAppend(id)
                downloaded = 0
                total = null
                contentType = null
                verifyingResume = false
            }

            // Phase A: the first chunk, fetched alone. `fullUntotaledFirst` is
            // set when it came back FULL with no reported total: Phase B′ must
            // keep pulling until a short/empty chunk proves EOF.
            var fullUntotaledFirst = false
            phaseA@ while (true) {
                currentCoroutineContext().ensureActive()
                val t0 = total
                if (t0 != null) {
                    if (downloaded == t0) break // resume partial already complete
                    if (downloaded > t0) {
                        // An impossible partial (e.g. mis-offset appended bytes)
                        // can never complete — start over.
                        DeviceLog.warn(
                            "device-download $id: staged partial ($downloaded) larger than server copy ($t0); restarting"
                        )
                        restartFromScratch()
                        continue@phaseA
                    }
                }
                val chunk = withContext(Dispatchers.IO) {
                    fetchChunkRetrying(base, token, start = downloaded, end = downloaded + chunkLen - 1)
                }

                if (verifyingResume) {
                    verifyingResume = false
                    // Accept the resume only if the server still range-serves
                    // the SAME file (total matches what we staged, or none staged).
                    if (chunk.total != null && (total == null || total == chunk.total)) {
                        total = chunk.total
                    } else {
                        DeviceLog.warn(
                            "device-download $id: resume partial no longer matches server copy; restarting"
                        )
                        restartFromScratch()
                        continue@phaseA
                    }
                }
                if (downloaded == 0L) {
                    total = chunk.total
                    contentType = chunk.contentType
                }
                saveMeta(id, PartialMeta(contentType, total))
                if (chunk.bytes.isEmpty()) break // server has no more to give
                append(handle, chunk.bytes)
                downloaded += chunk.bytes.size
                fullUntotaledFirst = total == null && chunk.bytes.size.toLong() == chunkLen
                publishProgress(id, downloaded, total)
                break
            }

            // Phase B′: sequential ranges until a short/empty chunk marks EOF,
            // so a sliced-but-untotaled response can never commit as a truncated
            // "complete" file. Hands off to Phase B if a total appears mid-stream.
            while (fullUntotaledFirst) {
                currentCoroutineContext().ensureActive()
                val chunk = withContext(Dispatchers.IO) {
                    fetchChunkRetrying(base, token, start = downloaded, end = downloaded + chunkLen - 1)
                }
                if (chunk.total != null) {
                    total = chunk.total
                    saveMeta(id, PartialMeta(contentType, total))
                }
                if (chunk.bytes.isEmpty()) break
                append(handle, chunk.bytes)
                downloaded += chunk.bytes.size
                publishProgress(id, downloaded, total)
                fullUntotaledFirst = total == null && chunk.bytes.size.toLong() == chunkLen
            }

            // Phase B: remaining chunks, up to `parallelism` in flight. Writes
            // land strictly in byte order via a reorder buffer; on any fetch
            // failure the scope cancels and the contiguous bytes written so far
            // stay in the durable partial for a later resume.
            val t = total
            if (t != null && downloaded < t) {
                val window = max(1, parallelism)
                val fetchDispatcher = Dispatchers.IO.limitedParallelism(window)
                var nextWrite = downloaded
                var nextFetch = downloaded
                var received = downloaded // bytes pulled (any order) — drives progress
                val buffer = mutableMapOf<Long, ByteArray>()
                val writeHandle = handle
                coroutineScope {
                    val completions = Channel<Pair<Long, Result<Chunk>>>(Channel.UNLIMITED)
                    var inflight = 0
                    while (nextWrite < t) {
                        // Keep the window full, counting BOTH in-flight requests
                        // AND completed-but-unwritten chunks, so a slow head
                        // chunk can't let later arrivals pile up unbounded.
                        while (inflight + buffer.size < window && nextFetch < t) {
                            val start = nextFetch
                            val end = min(start + chunkLen - 1, t - 1)
                            nextFetch = end + 1
                            inflight += 1
                            launch(fetchDispatcher) {
                                val result = try {
                                    Result.success(fetchFullRange(base, token, start, end))
                                } catch (e: CancellationException) {
                                    throw e
                                } catch (e: Exception) {
                                    Result.failure(e)
                                }
                                completions.send(start to result)
                            }
                        }
                        // Drain completions until the chunk we must write next is
                        // buffered. `nextWrite`'s chunk is always issued before
                        // any later one, so completions keep coming while it's due.
                        while (buffer[nextWrite] == null) {
                            if (inflight == 0) {
                                throw DownloadError.Failed("download stalled at $nextWrite/$t bytes")
                            }
                            val (start, result) = completions.receive()
                            inflight -= 1
                            val chunk = result.getOrThrow()
                            // The server's copy changed size mid-download: these
                            // bytes are from a different file — bail; the resume's
                            // total check then discards the stale partial.
                            if (chunk.total != null && chunk.total != t) {
                                throw DownloadError.Failed(
                                    "server copy changed during download (was $t, now ${chunk.total})")
                            }
                            received += chunk.bytes.size
                            publishProgress(id, received, total)
                            buffer[start] = chunk.bytes
                        }
                        val bytes = buffer.remove(nextWrite)
                        if (bytes == null || bytes.isEmpty()) break
                        append(writeHandle, bytes)
                        downloaded += bytes.size
                        nextWrite += bytes.size
                    }
                }
            }

            // A short read is refused outright — a truncated download can never
            // masquerade as a complete one.
            val finalTotal = total
            if (finalTotal != null && downloaded != finalTotal) {
                throw DownloadError.Failed("incomplete download ($downloaded/$finalTotal bytes)")
            }
            if (downloaded == 0L) {
                throw DownloadError.Failed("empty download")
            }
        } finally {
            runCatching { handle.close() }
        }
    }

    // ── streaming strategy (web download_audio_streaming) ────────────────

    // NO chunking: a single open-ended request streamed to the partial in
    // bounded flushes. A resume is valid only if the server range-served from
    // exactly the staged offset AND the size still matches — otherwise the
    // partial is discarded and the download restarts from byte 0 (at most once).
    private suspend fun downloadStreaming(id: Int) {
        val base = core.audioUrl(episodeId = id) ?: throw DownloadError.Failed("signed out")
        val token = core.apiToken
        var fromScratch = false
        while (true) {
            currentCoroutineContext().ensureActive()
            if (fromScratch) discardPartial(id)
            var downloaded = partialLength(id)
            val stagedMeta = if (downloaded > 0) loadMeta(id) else null
            if (downloaded > 0) {
                DeviceLog.info("device-download $id: resuming (streaming) at $downloaded")
            }

            val request = Request.Builder().url(base).apply {
                if (token != null) header("Authorization", "Bearer $token")
                if (downloaded > 0) header("Range", "bytes=$downloaded-")
            }.build()
            val response = streamClient.newCall(request).await()
            val servedFrom: Long
            val total: Long?
            when (response.code) {
                206 -> {
                    val (start, rangeTotal) = parseContentRange(response.header("Content-Range"))
                    servedFrom = start ?: downloaded
                    total = rangeTotal
                }
                200 -> {
                    servedFrom = 0
                    val length = response.body?.contentLength() ?: -1L
                    total = if (length > 0) length else null
                }
                416 -> {
                    response.close()
                    // The staged partial is already the whole file.
                    if (downloaded > 0 && stagedMeta?.total == downloaded) return
                    throw DownloadError.Http(416)
                }
                else -> {
                    val code = response.code
                    response.close()
                    throw DownloadError.Http(code)
                }
            }

            if (downloaded > 0) {
                val sizeOk =
                    total == null || stagedMeta?.total == null || total == stagedMeta?.total
                if (servedFrom != downloaded || !sizeOk) {
                    DeviceLog.warn(
                        "device-download $id: streaming resume no longer matches server copy; restarting"
                    )
                    response.close()
                    fromScratch = true
                    continue
                }
            }

            val contentType =
                if (downloaded > 0) stagedMeta?.contentType else response.header("Content-Type")
            val finalTotal = total ?: stagedMeta?.total
            saveMeta(id, PartialMeta(contentType, finalTotal))

            downloaded = withContext(Dispatchers.IO) {
                var written = downloaded
                val handle = openPartialForAppend(id)
                try {
                    response.use { resp ->
                        val body = resp.body ?: throw DownloadError.Failed("bad server response")
                        val source = body.byteStream()
                        val buf = ByteArray(64 * 1024)
                        var sinceFlush = 0
                        try {
                            while (true) {
                                val read = source.read(buf)
                                if (read < 0) break
                                handle.write(buf, 0, read)
                                written += read
                                sinceFlush += read
                                if (sinceFlush >= STREAM_FLUSH_BYTES) {
                                    handle.flush()
                                    sinceFlush = 0
                                    publishProgress(id, written, finalTotal)
                                    currentCoroutineContext().ensureActive()
                                }
                            }
                        } catch (e: IOException) {
                            // A cancel aborts the socket as an IOException —
                            // surface it as the cancellation it is.
                            currentCoroutineContext().ensureActive()
                            throw e
                        }
                        handle.flush()
                        publishProgress(id, written, finalTotal)
                    }
                } finally {
                    runCatching { handle.close() }
                }
                written
            }

            if (finalTotal != null && downloaded != finalTotal) {
                throw DownloadError.Failed("incomplete download ($downloaded/$finalTotal bytes)")
            }
            if (downloaded == 0L) {
                throw DownloadError.Failed("empty download")
            }
            return
        }
    }

    // ── chunk fetching (always invoked on an IO dispatcher) ──────────────

    private class Chunk(
        val bytes: ByteArray,
        // Full file size (Content-Range denominator / 200 Content-Length).
        val total: Long?,
        // Where the response body actually starts (0 for a range-ignoring 200).
        val servedFrom: Long,
        val contentType: String?,
    )

    private sealed class DownloadError(val messageText: String) : Exception(messageText) {
        // Body starts elsewhere than the requested offset — the bytes must
        // never be written there (discard-and-restart recovery in downloadChunked).
        class OffsetMismatch(val requested: Long, val servedFrom: Long) :
            DownloadError(
                "server ignored the requested byte range (asked for offset $requested, served $servedFrom)")

        class Http(val status: Int) : DownloadError("HTTP $status")

        class Failed(message: String) : DownloadError(message)
    }

    // Fetch one chunk with bounded exponential-backoff retries. Every response
    // is verified to start at the requested offset — a range-ignoring peer
    // yields OffsetMismatch immediately (no retries); auth/gone fail fast.
    private suspend fun fetchChunkRetrying(
        base: HttpUrl, token: String?, start: Long, end: Long
    ): Chunk {
        var backoff = BACKOFF_INITIAL_MS
        var lastError: Exception = DownloadError.Failed("no attempts")
        for (attempt in 1..CHUNK_RETRIES) {
            currentCoroutineContext().ensureActive()
            try {
                val chunk = fetchChunk(base, token, start, end)
                if (chunk.servedFrom != start) {
                    throw DownloadError.OffsetMismatch(requested = start, servedFrom = chunk.servedFrom)
                }
                return chunk
            } catch (e: DownloadError.OffsetMismatch) {
                throw e
            } catch (e: DownloadError.Http) {
                if (e.status in intArrayOf(401, 403, 404, 410)) throw e // won't succeed on retry
                lastError = e
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                // A cancelled call surfaces as IOException — re-raise the cancel.
                currentCoroutineContext().ensureActive()
                lastError = e
            }
            if (attempt < CHUNK_RETRIES) {
                delay(backoff)
                backoff = min(backoff * 2, BACKOFF_MAX_MS)
            }
        }
        throw lastError
    }

    // Fetch the COMPLETE byte range [start, end] (inclusive), re-requesting the
    // remainder on a short 206 — Phase B's reorder buffer requires full spans.
    // A zero-byte response for a range the server should still hold is a
    // genuine error (not an infinite re-request loop).
    private suspend fun fetchFullRange(
        base: HttpUrl, token: String?, start: Long, end: Long
    ): Chunk {
        val want = (end - start + 1).toInt()
        var acc = ByteArray(0)
        var total: Long? = null
        var contentType: String? = null
        while (acc.size < want) {
            val next = start + acc.size
            val chunk = fetchChunkRetrying(base, token, start = next, end = end)
            if (chunk.total != null) total = chunk.total
            if (contentType == null) contentType = chunk.contentType
            if (chunk.bytes.isEmpty()) {
                throw DownloadError.Failed(
                    "short range read: got ${acc.size} of $want bytes for [$start,$end]")
            }
            acc = if (acc.isEmpty()) chunk.bytes else acc + chunk.bytes
        }
        // Defensive: an over-serving server can't desync the write cursor.
        if (acc.size > want) acc = acc.copyOf(want)
        return Chunk(acc, total, servedFrom = start, contentType = contentType)
    }

    private suspend fun fetchChunk(
        base: HttpUrl, token: String?, start: Long, end: Long
    ): Chunk {
        val request = Request.Builder().url(base).apply {
            if (token != null) header("Authorization", "Bearer $token")
            header("Range", "bytes=$start-$end")
        }.build()
        chunkClient.newCall(request).await().use { response ->
            val contentType = response.header("Content-Type")
            return when (response.code) {
                206 -> {
                    val (rangeStart, total) = parseContentRange(response.header("Content-Range"))
                    Chunk(
                        response.body?.bytes() ?: ByteArray(0), total,
                        servedFrom = rangeStart ?: start, contentType = contentType)
                }
                200 -> {
                    // The server ignored the Range: the whole file from byte 0.
                    // The caller's served-offset check decides whether that's a
                    // byte-0 request (fine) or a nonzero-offset mismatch.
                    val bytes = response.body?.bytes() ?: ByteArray(0)
                    Chunk(bytes, bytes.size.toLong(), servedFrom = 0, contentType = contentType)
                }
                416 -> {
                    // Requested past the end — nothing more to serve. The total
                    // (if any) comes from "Content-Range: bytes */<total>".
                    val (_, total) = parseContentRange(response.header("Content-Range"))
                    Chunk(ByteArray(0), total, servedFrom = start, contentType = null)
                }
                else -> throw DownloadError.Http(response.code)
            }
        }
    }

    // ── commit + staging files ───────────────────────────────────────────

    // The `.partial.meta` sidecar: content type + total, so a resume keeps the
    // same type/expectation. The `downloaded` offset is NOT stored — it's the
    // live `.partial` file length, which can't drift from the bytes on disk.
    @Serializable
    private data class PartialMeta(val contentType: String? = null, val total: Long? = null)

    // Bytes become the playable local copy only here, after a verified-complete
    // download: promote the partial onto `<id>.<ext>` (extension from the
    // content type) and sweep the sidecar + any previous differently-named copy.
    private fun commit(id: Int) {
        val contentType = loadMeta(id)?.contentType
        val final = File(dir, "$id.${ext(contentType)}")
        committedFile(id)?.delete()
        final.delete()
        if (!partialFile(id).renameTo(final)) {
            throw DownloadError.Failed("couldn't move the finished download into place")
        }
        metaFile(id).delete()
        putState(id, State.Downloaded)
        DeviceLog.info("device-download $id: complete (${final.length()} bytes, ${final.name})")
    }

    // The committed audio file for this id, any extension.
    private fun committedFile(id: Int): File? {
        val prefix = "$id."
        return dir.list().orEmpty()
            .firstOrNull { it.startsWith(prefix) && !isPartialArtifact(it) }
            ?.let { File(dir, it) }
    }

    private fun openPartialForAppend(id: Int): FileOutputStream {
        val partial = partialFile(id)
        if (!partial.exists()) partial.createNewFile()
        return FileOutputStream(partial, true)
    }

    private suspend fun append(handle: FileOutputStream, bytes: ByteArray) {
        withContext(Dispatchers.IO) { handle.write(bytes) }
    }

    private fun partialLength(id: Int): Long = partialFile(id).length()

    private fun discardPartial(id: Int) {
        partialFile(id).delete()
        metaFile(id).delete()
    }

    private fun loadMeta(id: Int): PartialMeta? {
        val file = metaFile(id)
        if (!file.exists()) return null
        return runCatching {
            metaJson.decodeFromString(PartialMeta.serializer(), file.readText())
        }.getOrNull()
    }

    private fun saveMeta(id: Int, meta: PartialMeta) {
        runCatching {
            metaFile(id).writeText(metaJson.encodeToString(PartialMeta.serializer(), meta))
        }
    }

    // ── helpers ──────────────────────────────────────────────────────────

    // State-map writes are single-threaded on the Main scope, so check-then-set
    // stays serialized against pause/remove (the iOS main-actor guarantee).
    private fun putState(id: Int, state: State?) {
        states = if (state == null) states - id else states + (id to state)
    }

    private fun publishProgress(id: Int, downloaded: Long, total: Long?) {
        scope.launch {
            // Don't stomp a pause/removal that landed while a write was in flight.
            if (states[id] !is State.Downloading) return@launch
            if (total != null && total > 0) {
                putState(id, State.Downloading(min(downloaded.toDouble() / total.toDouble(), 1.0)))
            }
        }
    }

    private fun currentProgress(id: Int): Double =
        when (val state = states[id]) {
            is State.Downloading -> state.progress
            is State.Paused -> state.progress
            else -> 0.0
        }

    private fun rememberOnDevice(episode: EpisodeData) {
        if (onDevice.any { it.id == episode.id }) return
        onDevice = listOf(episode) + onDevice
        persistList()
    }

    private fun persistList() {
        val snapshot = onDevice
        scope.launch { core.store?.save(snapshot, CacheKey.deviceDownloads) }
    }

    private fun partialFile(id: Int): File = File(dir, "$id.partial")

    private fun metaFile(id: Int): File = File(dir, "$id.partial.meta")
}
