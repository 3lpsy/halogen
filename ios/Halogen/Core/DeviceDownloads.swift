import Foundation
import Observation

/// Device downloads — audio bytes cached ON THIS DEVICE (web download.rs).
/// Phase 1: durable TriggerDownload op + poll until the server copy exists.
/// Phase 2: Range-chunk (or stream) into a `.partial` whose length IS the
/// resume point, committing ONLY a verified-complete file.
@MainActor
@Observable
final class DeviceDownloads {
    enum State: Equatable {
        case none
        /// Phase 1: the triggered server download is being awaited. Rows
        /// render the CLOUD ring (server progress) for this phase.
        case waitingServer
        case downloading(progress: Double)
        case paused(progress: Double)
        /// Terminal failure (server couldn't fetch, offline, corrupt pull…) —
        /// surfaced instead of masquerading as a pause. Tap retries.
        case failed(message: String)
        case downloaded
    }

    // Web parity constants (crates/ui-svc-sync download.rs).
    /// How long to wait for the SERVER to finish fetching its copy
    /// (tries × interval ≈ 2 minutes).
    private static let serverPollTries = 40
    private static let serverPollIntervalSecs: Double = 3
    /// Consecutive transport-error polls that mean "offline" rather than
    /// "server is slow to fetch".
    private static let serverPollOfflineGiveup = 5
    /// Per-chunk attempts before the download gives up. These three are
    /// nonisolated: the chunk-fetch loop reads them off the main actor, and
    /// immutable Sendable constants are safe from any isolation.
    private nonisolated static let chunkRetries = 5
    private nonisolated static let backoffInitialMs: UInt64 = 500
    private nonisolated static let backoffMaxMs: UInt64 = 8_000
    /// Bytes accumulated before a flush in the streaming (no-chunking) path.
    private static let streamFlushBytes = 4 * 1024 * 1024

    private unowned let core: HalogenCore
    /// Episode id → live state. Finished episodes are also discoverable from
    /// disk after relaunch (scan in `load`).
    private(set) var states: [Int32: State] = [:]
    /// Episodes on device, newest-added first (the Downloads page's facet).
    private(set) var onDevice: [EpisodeData] = []

    private var tasks: [Int32: Task<Void, Never>] = [:]
    private let dir: URL

    init(core: HalogenCore, namespace: String) {
        self.core = core
        let support = try! FileManager.default.url(
            for: .applicationSupportDirectory, in: .userDomainMask,
            appropriateFor: nil, create: true)
        dir =
            support
            .appendingPathComponent("halogen-client", isDirectory: true)
            .appendingPathComponent(namespace, isDirectory: true)
            .appendingPathComponent("audio", isDirectory: true)
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
    }

    /// Rebuild state from disk (finished files + partials) and the cached
    /// episode metadata list.
    func load() async {
        if let cached = await core.store?.load([EpisodeData].self, key: CacheKey.deviceDownloads) {
            onDevice = cached
        }
        let files = (try? FileManager.default.contentsOfDirectory(atPath: dir.path)) ?? []
        for file in files {
            guard let id = Int32(file.split(separator: ".").first ?? "") else { continue }
            if file.hasSuffix(".partial") {
                if states[id] == nil {
                    // Seed the resume progress from the staged offset + total
                    // (web: resume_partial_downloads' percent seeding).
                    let staged = partialLength(id)
                    let total = loadMeta(id)?.total ?? 0
                    let progress = total > 0 ? min(Double(staged) / Double(total), 1) : 0
                    states[id] = .paused(progress: progress)
                }
            } else if !Self.isPartialArtifact(file) {
                states[id] = .downloaded
            }
        }
    }

    func state(of episodeId: Int32) -> State {
        states[episodeId] ?? .none
    }

    /// The local file to play from, when fully downloaded (any extension —
    /// commit names the file from the download's content type).
    func localURL(episodeId: Int32) -> URL? {
        committedURL(episodeId)
    }

    /// (file count, total bytes) for the purge screen — finals + partials.
    var stats: (count: Int, bytes: UInt64) {
        let files = (try? FileManager.default.contentsOfDirectory(atPath: dir.path)) ?? []
        var bytes: UInt64 = 0
        for file in files {
            let attrs = try? FileManager.default.attributesOfItem(
                atPath: dir.appendingPathComponent(file).path)
            bytes += (attrs?[.size] as? UInt64) ?? 0
        }
        return (files.count, bytes)
    }

    /// Delete every local copy + partial (the purge screen).
    func removeAll() {
        for id in Array(tasks.keys) { pause(id) }
        let files = (try? FileManager.default.contentsOfDirectory(atPath: dir.path)) ?? []
        for file in files {
            try? FileManager.default.removeItem(at: dir.appendingPathComponent(file))
        }
        states.removeAll()
        onDevice.removeAll()
        persistList()
    }

    // MARK: - control

    /// Boot auto-resume (web resume_partial_downloads): staged partials
    /// continue without a manual tap when online; failures stay parked.
    func resumePartials() {
        guard !core.isOffline else { return }
        for episode in onDevice {
            if case .paused = state(of: episode.id) {
                download(episode)
            }
        }
    }

    func download(_ episode: EpisodeData) {
        let id = episode.id
        switch state(of: id) {
        case .downloaded, .downloading, .waitingServer: return
        default: break
        }
        // Fail fast when known-offline: the device pulls the SERVER's copy
        // (web sync-status gate). `.unknown` deliberately proceeds — local-first.
        if core.isOffline {
            states[id] = .failed(
                message: "You're offline — can't download to this device right now.")
            return
        }
        states[id] = .downloading(progress: currentProgress(id))
        rememberOnDevice(episode)
        let token = UUID()
        runTokens[id] = token
        tasks[id] = Task { [weak self] in
            await self?.run(episode, token: token)
        }
    }

    /// Identity of the CURRENT run per episode: a cancelled run's cleanup must
    /// not clobber a restarted download's handle (pause/remove became no-ops).
    private var runTokens: [Int32: UUID] = [:]

    func pause(_ episodeId: Int32) {
        tasks[episodeId]?.cancel()
        tasks[episodeId] = nil
        switch state(of: episodeId) {
        case .downloading(let p):
            states[episodeId] = .paused(progress: p)
        case .waitingServer:
            // Nothing partial to keep; the server-side fetch keeps running.
            // (nil drops the entry; state(of:) reads a missing key as .none.)
            states[episodeId] = nil
        default:
            break
        }
    }

    func remove(_ episodeId: Int32) {
        pause(episodeId)
        if let committed = committedURL(episodeId) {
            try? FileManager.default.removeItem(at: committed)
        }
        discardPartial(episodeId)
        states[episodeId] = nil
        onDevice.removeAll { $0.id == episodeId }
        persistList()
    }

    // MARK: - the download task

    private func run(_ episode: EpisodeData, token: UUID) async {
        let id = episode.id
        defer {
            // Only THIS run's registration — a restart has its own token.
            if runTokens[id] == token {
                tasks[id] = nil
                runTokens[id] = nil
            }
        }

        // Phase 1: make sure the server holds its copy — the audio endpoint
        // 404s otherwise (server audio.rs serves only Downloaded rows).
        let serverReady =
            core.models?.serverDownloads.isDownloaded(episode)
            ?? (episode.download_status == .downloaded)
        if !serverReady {
            states[id] = .waitingServer
            guard await waitForServerCopy(id) else { return }
            states[id] = .downloading(progress: currentProgress(id))
        }
        if Task.isCancelled { return }

        // Phase 2: pull the bytes. On failure the durable partial is KEPT
        // (invisible to localURL) for the next manual retry to resume.
        let chunkKiB = core.models?.prefs.prefs.downloadChunkKiB
            ?? ClientPrefs.default.downloadChunkKiB
        let parallelism = max(1, core.models?.prefs.prefs.downloadParallelism ?? 1)
        do {
            if chunkKiB > 0 {
                try await downloadChunked(
                    id, chunkLen: UInt64(max(64, chunkKiB)) * 1024, parallelism: parallelism)
            } else {
                try await downloadStreaming(id)
            }
            try commit(id)
        } catch is CancellationError {
            return
        } catch let error as URLError where error.code == .cancelled {
            return
        } catch {
            let message = (error as? DownloadError)?.message ?? error.localizedDescription
            DeviceLog.warn("device-download \(id): \(message)")
            states[id] = .failed(message: message)
        }
    }

    /// Web download.rs phase 1: durable TriggerDownload op, then poll until the
    /// server copy is Downloaded — bailing early on manual offline, a terminal
    /// status, or repeated transport errors (false = failure state was set).
    private func waitForServerCopy(_ id: Int32) async -> Bool {
        // Durable + idempotent server-side; survives offline and restarts.
        await core.outbox?.enqueue(.triggerDownload(episodeId: id))
        // Track the server run too so the waiting row's cloud ring shows the
        // server's live progress (web: the cloud phase of Download & Play).
        core.models?.serverDownloads.watch(id)
        var consecutiveErrors = 0
        for _ in 0..<Self.serverPollTries {
            if Task.isCancelled { return false }
            if core.connection.manualOffline {
                states[id] = .failed(message: "Cancelled — you went offline.")
                return false
            }
            do {
                let fresh = try await core.episodeDetail(id: id)
                consecutiveErrors = 0
                switch fresh.download_status {
                case .downloaded:
                    return true
                case .downloadError, .downloadUnauthorized, .downloadRemoteNotFound,
                    .downloadBroken:
                    // Terminal: stop waiting and surface the error rather than
                    // polling until the budget runs out.
                    DeviceLog.warn(
                        "device-download \(id): server fetch ended \(fresh.download_status.rawValue)"
                    )
                    states[id] = .failed(message: "The server couldn't fetch the episode.")
                    return false
                default:
                    break
                }
            } catch {
                consecutiveErrors += 1
                if consecutiveErrors >= Self.serverPollOfflineGiveup {
                    states[id] = .failed(
                        message: "You appear to be offline — can't reach the server to download.")
                    return false
                }
            }
            try? await Task.sleep(for: .seconds(Self.serverPollIntervalSecs))
        }
        states[id] = .failed(
            message: consecutiveErrors > 0
                ? "You appear to be offline — can't reach the server to download."
                : "Timed out waiting for the server download.")
        return false
    }

    // MARK: - chunked strategy (web download_audio_chunked)

    /// One offset-mismatch discards the partial and re-runs from scratch; a
    /// second means the peer never honors ranges — fall back to streaming,
    /// which handles a 200-from-byte-0 correctly. Never commits stitched bytes.
    private func downloadChunked(_ id: Int32, chunkLen: UInt64, parallelism: Int) async throws {
        do {
            try await chunkedAttempt(id, chunkLen: chunkLen, parallelism: parallelism)
        } catch DownloadError.offsetMismatch(let requested, let servedFrom) {
            DeviceLog.warn(
                "device-download \(id): chunk served from \(servedFrom), asked \(requested); restarting from scratch"
            )
            discardPartial(id)
            do {
                try await chunkedAttempt(id, chunkLen: chunkLen, parallelism: parallelism)
            } catch DownloadError.offsetMismatch {
                DeviceLog.warn(
                    "device-download \(id): server never honors byte ranges; falling back to streaming"
                )
                discardPartial(id)
                try await downloadStreaming(id)
            }
        }
    }

    /// One pass of the chunked download: resume verification → Phase A → B′ →
    /// B → final completeness check (mirrors download_audio_chunked_attempt).
    private func chunkedAttempt(_ id: Int32, chunkLen: UInt64, parallelism: Int) async throws {
        guard let base = core.audioURL(episodeId: id) else {
            throw DownloadError.failed("signed out")
        }
        let token = core.apiToken

        var handle = try openPartialForAppend(id)
        defer { try? handle.close() }
        var downloaded = partialLength(id)
        var contentType = downloaded > 0 ? loadMeta(id)?.contentType : nil
        var total: UInt64? = downloaded > 0 ? loadMeta(id)?.total : nil
        // Validate the staged partial against the server copy before trusting
        // the bytes already on disk.
        var verifyingResume = downloaded > 0
        if verifyingResume {
            DeviceLog.info("device-download \(id): resuming at \(downloaded) bytes")
        }

        /// Discard the partial and start over as a fresh download.
        func restartFromScratch() throws {
            try? handle.close()
            discardPartial(id)
            handle = try openPartialForAppend(id)
            downloaded = 0
            total = nil
            contentType = nil
            verifyingResume = false
        }

        // ── Phase A: the first chunk, fetched alone ──────────────────────
        // Set when the first chunk came back FULL with no reported total:
        // Phase B′ must pull until a short/empty chunk proves EOF.
        var fullUntotaledFirst = false
        phaseA: while true {
            try Task.checkCancellation()
            if let t = total {
                if downloaded == t { break }  // resume partial already complete
                if downloaded > t {
                    // An impossible partial (e.g. an earlier run appended
                    // mis-offset bytes) can never complete — start over.
                    DeviceLog.warn(
                        "device-download \(id): staged partial (\(downloaded)) larger than server copy (\(t)); restarting"
                    )
                    try restartFromScratch()
                    continue
                }
            }
            let chunk = try await Self.fetchChunkRetrying(
                base: base, token: token, start: downloaded, end: downloaded + chunkLen - 1)

            if verifyingResume {
                verifyingResume = false
                // Accept the resume only if the server still range-serves the
                // SAME file (total matches what we staged, or none was staged).
                if let serverTotal = chunk.total, total == nil || total == serverTotal {
                    total = serverTotal
                } else {
                    DeviceLog.warn(
                        "device-download \(id): resume partial no longer matches server copy; restarting"
                    )
                    try restartFromScratch()
                    continue
                }
            }
            if downloaded == 0 {
                total = chunk.total
                contentType = chunk.contentType
            }
            saveMeta(id, PartialMeta(contentType: contentType, total: total))
            if chunk.bytes.isEmpty { break }  // server has no more to give
            try handle.write(contentsOf: chunk.bytes)
            downloaded += UInt64(chunk.bytes.count)
            fullUntotaledFirst = total == nil && UInt64(chunk.bytes.count) == chunkLen
            publishProgress(id, downloaded, total)
            break
        }

        // ── Phase B′: unknown-total continuation ─────────────────────────
        // Sequential ranges until a short/empty chunk marks EOF (an untotaled
        // response can't commit truncated); hands off to B if a total appears.
        while fullUntotaledFirst {
            try Task.checkCancellation()
            let chunk = try await Self.fetchChunkRetrying(
                base: base, token: token, start: downloaded, end: downloaded + chunkLen - 1)
            if chunk.total != nil {
                total = chunk.total
                saveMeta(id, PartialMeta(contentType: contentType, total: total))
            }
            if chunk.bytes.isEmpty { break }
            try handle.write(contentsOf: chunk.bytes)
            downloaded += UInt64(chunk.bytes.count)
            publishProgress(id, downloaded, total)
            fullUntotaledFirst = total == nil && UInt64(chunk.bytes.count) == chunkLen
        }

        // ── Phase B: remaining chunks, up to `parallelism` in flight ─────
        // Writes land strictly in byte order via a reorder buffer; a fetch
        // failure cancels the group, keeping contiguous bytes for resume.
        if let t = total, downloaded < t {
            let window = max(1, parallelism)
            var nextWrite = downloaded
            var nextFetch = downloaded
            var received = downloaded  // bytes pulled (any order) — drives progress
            var buffer: [UInt64: Data] = [:]
            let writeHandle = handle
            try await withThrowingTaskGroup(of: (UInt64, Chunk).self) { group in
                var inflight = 0
                while nextWrite < t {
                    // The window counts BOTH in-flight AND completed-but-unwritten
                    // chunks, so a slow head chunk can't pile up arrivals unbounded.
                    while inflight + buffer.count < window && nextFetch < t {
                        let start = nextFetch
                        let end = min(start + chunkLen - 1, t - 1)
                        nextFetch = end + 1
                        inflight += 1
                        group.addTask {
                            (
                                start,
                                try await Self.fetchFullRange(
                                    base: base, token: token, start: start, end: end)
                            )
                        }
                    }
                    // Drain completions until the next-write chunk is buffered;
                    // it's always issued first, so the group stays non-empty.
                    while buffer[nextWrite] == nil {
                        guard let (start, chunk) = try await group.next() else {
                            throw DownloadError.failed(
                                "download stalled at \(nextWrite)/\(t) bytes")
                        }
                        inflight -= 1
                        // Server copy changed size mid-download — bail; the
                        // resume's total check then discards the stale partial.
                        if let ct = chunk.total, ct != t {
                            throw DownloadError.failed(
                                "server copy changed during download (was \(t), now \(ct))")
                        }
                        received += UInt64(chunk.bytes.count)
                        publishProgress(id, received, total)
                        buffer[start] = chunk.bytes
                    }
                    guard let bytes = buffer.removeValue(forKey: nextWrite), !bytes.isEmpty
                    else { break }
                    try writeHandle.write(contentsOf: bytes)
                    downloaded += UInt64(bytes.count)
                    nextWrite += UInt64(bytes.count)
                }
            }
        }

        // A short read is refused outright — a truncated download can never
        // masquerade as a complete one.
        if let t = total, downloaded != t {
            throw DownloadError.failed("incomplete download (\(downloaded)/\(t) bytes)")
        }
        if downloaded == 0 {
            throw DownloadError.failed("empty download")
        }
    }

    // MARK: - streaming strategy (web download_audio_streaming)

    /// NO chunking: one open-ended request streamed to the partial in bounded
    /// flushes. A resume is valid only if range-served from exactly the staged
    /// offset AND the size still matches — else restart from byte 0 (once).
    private func downloadStreaming(_ id: Int32) async throws {
        guard let base = core.audioURL(episodeId: id) else {
            throw DownloadError.failed("signed out")
        }
        let token = core.apiToken
        var fromScratch = false
        while true {
            try Task.checkCancellation()
            if fromScratch { discardPartial(id) }
            var downloaded = partialLength(id)
            let stagedMeta = downloaded > 0 ? loadMeta(id) : nil
            if downloaded > 0 {
                DeviceLog.info("device-download \(id): resuming (streaming) at \(downloaded)")
            }

            var request = URLRequest(url: base)
            request.timeoutInterval = 60
            if let token {
                request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
            }
            if downloaded > 0 {
                request.setValue("bytes=\(downloaded)-", forHTTPHeaderField: "Range")
            }
            let (body, response) = try await URLSession.shared.bytes(for: request)
            guard let http = response as? HTTPURLResponse else {
                throw DownloadError.failed("bad server response")
            }
            let servedFrom: UInt64
            var total: UInt64?
            switch http.statusCode {
            case 206:
                let range = Self.parseContentRange(http.value(forHTTPHeaderField: "Content-Range"))
                servedFrom = range.start ?? downloaded
                total = range.total
            case 200:
                servedFrom = 0
                total = http.expectedContentLength > 0
                    ? UInt64(http.expectedContentLength) : nil
            case 416 where downloaded > 0 && stagedMeta?.total == downloaded:
                return  // the staged partial is already the whole file
            default:
                throw DownloadError.http(http.statusCode)
            }

            if downloaded > 0 {
                let sizeOk =
                    total == nil || stagedMeta?.total == nil || total == stagedMeta?.total
                if servedFrom != downloaded || !sizeOk {
                    DeviceLog.warn(
                        "device-download \(id): streaming resume no longer matches server copy; restarting"
                    )
                    fromScratch = true
                    continue
                }
            }

            let contentType =
                downloaded > 0
                ? stagedMeta?.contentType : http.value(forHTTPHeaderField: "Content-Type")
            let finalTotal = total ?? stagedMeta?.total
            saveMeta(id, PartialMeta(contentType: contentType, total: finalTotal))

            let handle = try openPartialForAppend(id)
            defer { try? handle.close() }
            var batch = Data(capacity: Self.streamFlushBytes)
            for try await byte in body {
                batch.append(byte)
                if batch.count >= Self.streamFlushBytes {
                    try handle.write(contentsOf: batch)
                    downloaded += UInt64(batch.count)
                    batch.removeAll(keepingCapacity: true)
                    publishProgress(id, downloaded, finalTotal)
                    try Task.checkCancellation()
                }
            }
            if !batch.isEmpty {
                try handle.write(contentsOf: batch)
                downloaded += UInt64(batch.count)
                publishProgress(id, downloaded, finalTotal)
            }

            if let t = finalTotal, downloaded != t {
                throw DownloadError.failed("incomplete download (\(downloaded)/\(t) bytes)")
            }
            if downloaded == 0 {
                throw DownloadError.failed("empty download")
            }
            return
        }
    }

    // MARK: - chunk fetching (nonisolated: runs off the main-actor loop)

    private struct Chunk: Sendable {
        let bytes: Data
        /// Full file size (Content-Range denominator / 200 Content-Length).
        let total: UInt64?
        /// Where the response body actually starts (0 for a range-ignoring 200).
        let servedFrom: UInt64
        let contentType: String?
    }

    private enum DownloadError: Error {
        /// Body starts elsewhere than the requested offset — must never be
        /// written there (discard-and-restart recovery in `downloadChunked`).
        case offsetMismatch(requested: UInt64, servedFrom: UInt64)
        case http(Int)
        case failed(String)

        var message: String {
            switch self {
            case .offsetMismatch(let requested, let servedFrom):
                return
                    "server ignored the requested byte range (asked for offset \(requested), served \(servedFrom))"
            case .http(let status):
                return "HTTP \(status)"
            case .failed(let message):
                return message
            }
        }
    }

    /// Fetch one chunk with bounded exponential-backoff retries. A response not
    /// starting at the requested offset is `.offsetMismatch` immediately (no
    /// retries); permanent failures (auth / gone) fail fast.
    private nonisolated static func fetchChunkRetrying(
        base: URL, token: String?, start: UInt64, end: UInt64
    ) async throws -> Chunk {
        var delay = backoffInitialMs
        var lastError: Error = DownloadError.failed("no attempts")
        for attempt in 1...chunkRetries {
            try Task.checkCancellation()
            do {
                let chunk = try await fetchChunk(base: base, token: token, start: start, end: end)
                guard chunk.servedFrom == start else {
                    throw DownloadError.offsetMismatch(
                        requested: start, servedFrom: chunk.servedFrom)
                }
                return chunk
            } catch let error as DownloadError {
                if case .offsetMismatch = error { throw error }
                if case .http(let status) = error, [401, 403, 404, 410].contains(status) {
                    throw error  // won't succeed on retry
                }
                lastError = error
            } catch let error as URLError where error.code == .cancelled {
                throw CancellationError()
            } catch is CancellationError {
                throw CancellationError()
            } catch {
                lastError = error
            }
            if attempt < chunkRetries {
                try await Task.sleep(for: .milliseconds(delay))
                delay = min(delay * 2, backoffMaxMs)
            }
        }
        throw lastError
    }

    /// Fetch the COMPLETE range `[start, end]`, re-requesting short 206 reads —
    /// Phase B's reorder buffer needs full spans. A zero-byte response for a
    /// range the server should hold is an error, not an endless re-request.
    private nonisolated static func fetchFullRange(
        base: URL, token: String?, start: UInt64, end: UInt64
    ) async throws -> Chunk {
        let want = Int(end - start + 1)
        var acc = Data(capacity: want)
        var total: UInt64?
        var contentType: String?
        while acc.count < want {
            let next = start + UInt64(acc.count)
            let chunk = try await fetchChunkRetrying(base: base, token: token, start: next, end: end)
            if chunk.total != nil { total = chunk.total }
            if contentType == nil { contentType = chunk.contentType }
            if chunk.bytes.isEmpty {
                throw DownloadError.failed(
                    "short range read: got \(acc.count) of \(want) bytes for [\(start),\(end)]")
            }
            acc.append(chunk.bytes)
        }
        // Defensive: an over-serving server can't desync the write cursor.
        if acc.count > want { acc = acc.prefix(want) }
        return Chunk(bytes: acc, total: total, servedFrom: start, contentType: contentType)
    }

    private nonisolated static func fetchChunk(
        base: URL, token: String?, start: UInt64, end: UInt64
    ) async throws -> Chunk {
        var request = URLRequest(url: base)
        request.timeoutInterval = 30
        if let token {
            request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        }
        request.setValue("bytes=\(start)-\(end)", forHTTPHeaderField: "Range")
        let (data, response) = try await URLSession.shared.data(for: request)
        guard let http = response as? HTTPURLResponse else {
            throw DownloadError.failed("bad server response")
        }
        let contentType = http.value(forHTTPHeaderField: "Content-Type")
        switch http.statusCode {
        case 206:
            let range = parseContentRange(http.value(forHTTPHeaderField: "Content-Range"))
            return Chunk(
                bytes: data, total: range.total, servedFrom: range.start ?? start,
                contentType: contentType)
        case 200:
            // Range ignored: the whole file from byte 0. The caller's
            // served-offset check decides acceptable (byte-0) vs mismatch.
            return Chunk(
                bytes: data, total: UInt64(data.count), servedFrom: 0, contentType: contentType)
        case 416:
            // Requested past the end — nothing more to serve. The total (if
            // any) comes from "Content-Range: bytes */<total>".
            let range = parseContentRange(http.value(forHTTPHeaderField: "Content-Range"))
            return Chunk(bytes: Data(), total: range.total, servedFrom: start, contentType: nil)
        default:
            throw DownloadError.http(http.statusCode)
        }
    }

    /// "bytes 0-1023/4096" → (0, 4096); "bytes */4096" → (nil, 4096).
    private nonisolated static func parseContentRange(_ value: String?) -> (
        start: UInt64?, total: UInt64?
    ) {
        guard let value else { return (nil, nil) }
        let spec = value.replacingOccurrences(of: "bytes", with: "")
            .trimmingCharacters(in: .whitespaces)
        let parts = spec.split(separator: "/", maxSplits: 1)
        let total = parts.count == 2 ? UInt64(parts[1]) : nil
        let start = parts.first.flatMap { $0.split(separator: "-").first.flatMap { UInt64($0) } }
        return (start, total)
    }

    // MARK: - commit + staging files

    /// The `.partial.meta` sidecar: content type + total, so a resume keeps the
    /// same type/expectation (web PartialMeta). The `downloaded` offset is NOT
    /// stored — it's the live `.partial` length, which can't drift from disk.
    private struct PartialMeta: Codable {
        var contentType: String?
        var total: UInt64?
    }

    /// Bytes become the playable local copy only here, after a verified-complete
    /// download: promote the partial onto `<id>.<ext>` (extension from the
    /// content type), sweeping the sidecar + any previous copy.
    private func commit(_ id: Int32) throws {
        let contentType = loadMeta(id)?.contentType
        let final = dir.appendingPathComponent("\(id).\(Self.ext(for: contentType))")
        if let previous = committedURL(id) {
            try? FileManager.default.removeItem(at: previous)
        }
        try? FileManager.default.removeItem(at: final)
        try FileManager.default.moveItem(at: partialURL(id), to: final)
        try? FileManager.default.removeItem(at: metaURL(id))
        states[id] = .downloaded
        let size =
            (try? FileManager.default.attributesOfItem(atPath: final.path)[.size] as? UInt64) ?? 0
        DeviceLog.info("device-download \(id): complete (\(size) bytes, \(final.lastPathComponent))")
    }

    /// File extension for a downloaded content type (web ext_for). Unknown or
    /// missing types fall back to mp3 — the common case.
    private static func ext(for contentType: String?) -> String {
        let mime = contentType?.split(separator: ";").first?
            .trimmingCharacters(in: .whitespaces).lowercased()
        switch mime {
        case "audio/mpeg", "audio/mp3": return "mp3"
        case "audio/mp4", "audio/x-m4a", "audio/aac": return "m4a"
        case "audio/ogg": return "ogg"
        case "audio/opus": return "opus"
        case "audio/wav", "audio/x-wav": return "wav"
        case "audio/flac": return "flac"
        default: return "mp3"
        }
    }

    /// True for `<id>.partial` / `<id>.partial.meta` — staging artifacts,
    /// never playable audio.
    private static func isPartialArtifact(_ name: String) -> Bool {
        name.hasSuffix(".partial") || name.hasSuffix(".partial.meta")
    }

    /// The committed audio file for this id, any extension.
    private func committedURL(_ id: Int32) -> URL? {
        let prefix = "\(id)."
        let files = (try? FileManager.default.contentsOfDirectory(atPath: dir.path)) ?? []
        return files.first { $0.hasPrefix(prefix) && !Self.isPartialArtifact($0) }
            .map { dir.appendingPathComponent($0) }
    }

    private func openPartialForAppend(_ id: Int32) throws -> FileHandle {
        let partial = partialURL(id)
        if !FileManager.default.fileExists(atPath: partial.path) {
            FileManager.default.createFile(atPath: partial.path, contents: nil)
        }
        let handle = try FileHandle(forWritingTo: partial)
        try handle.seekToEnd()
        return handle
    }

    private func partialLength(_ id: Int32) -> UInt64 {
        let attrs = try? FileManager.default.attributesOfItem(atPath: partialURL(id).path)
        return (attrs?[.size] as? UInt64) ?? 0
    }

    private func discardPartial(_ id: Int32) {
        try? FileManager.default.removeItem(at: partialURL(id))
        try? FileManager.default.removeItem(at: metaURL(id))
    }

    private func loadMeta(_ id: Int32) -> PartialMeta? {
        guard let data = try? Data(contentsOf: metaURL(id)) else { return nil }
        return try? JSONDecoder().decode(PartialMeta.self, from: data)
    }

    private func saveMeta(_ id: Int32, _ meta: PartialMeta) {
        if let data = try? JSONEncoder().encode(meta) {
            try? data.write(to: metaURL(id))
        }
    }

    // MARK: - helpers

    private func publishProgress(_ id: Int32, _ downloaded: UInt64, _ total: UInt64?) {
        // Don't stomp a pause/removal that landed while a write was in flight.
        guard case .downloading = states[id] ?? .none else { return }
        if let total, total > 0 {
            states[id] = .downloading(progress: min(Double(downloaded) / Double(total), 1))
        }
    }

    private func currentProgress(_ id: Int32) -> Double {
        if case .downloading(let p) = states[id] { return p }
        if case .paused(let p) = states[id] { return p }
        return 0
    }

    private func rememberOnDevice(_ episode: EpisodeData) {
        guard !onDevice.contains(where: { $0.id == episode.id }) else { return }
        onDevice.insert(episode, at: 0)
        persistList()
    }

    private func persistList() {
        let snapshot = onDevice
        Task { [store = core.store] in
            await store?.save(snapshot, key: CacheKey.deviceDownloads)
        }
    }

    private func partialURL(_ id: Int32) -> URL {
        dir.appendingPathComponent("\(id).partial")
    }

    private func metaURL(_ id: Int32) -> URL {
        dir.appendingPathComponent("\(id).partial.meta")
    }
}
