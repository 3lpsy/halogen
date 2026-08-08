import Foundation
import Observation

/// Live server-side download tracking: poll the per-episode progress endpoint
/// (~1.5s, web parity) until the tracker drops the entry, then confirm the
/// final status. Rows read `progress(of:)` (cloud ring) and `completed`.
@MainActor
@Observable
final class ServerDownloads {
    private unowned let core: HalogenCore

    /// episode id → 0…1 while the server download runs.
    private(set) var progress: [Int32: Double] = [:]
    /// Episodes confirmed DOWNLOADED after a tracked run (row-state override).
    private(set) var completed: Set<Int32> = []
    /// Episodes whose tracked run ended NOT downloaded (error surfaced).
    private(set) var failed: Set<Int32> = []
    /// Episodes optimistically removed from the server (the queued
    /// RemoveServerDownload op hasn't drained/refreshed yet) — row snapshots
    /// still say DOWNLOADED, this overlay wins (web: the optimistic status
    /// reset in the RemoveServerDownload command arm).
    private(set) var removed: Set<Int32> = []

    private var tasks: [Int32: Task<Void, Never>] = [:]

    init(core: HalogenCore) {
        self.core = core
    }

    func progress(of episodeId: Int32) -> Double? {
        progress[episodeId]
    }

    func isDownloaded(_ episode: EpisodeData) -> Bool {
        if removed.contains(episode.id) { return false }
        return completed.contains(episode.id)
            || (episode.download_status == .downloaded && !failed.contains(episode.id))
    }

    /// Optimistic local reset paired with an enqueued RemoveServerDownload op
    /// — the row flips immediately (offline too) instead of waiting for the
    /// drain + next refresh.
    func markRemovedLocally(_ episodeId: Int32) {
        removed.insert(episodeId)
        completed.remove(episodeId)
        progress[episodeId] = nil
    }

    /// Trigger the server download (idempotent server-side) and start polling.
    func download(_ episode: EpisodeData) {
        let id = episode.id
        guard tasks[id] == nil else { return }
        progress[id] = 0
        failed.remove(id)
        removed.remove(id)
        tasks[id] = Task { [weak self] in
            await self?.track(id)
        }
    }

    /// Watch an already-running server download (rows showing DOWNLOADING).
    func watch(_ episodeId: Int32) {
        guard tasks[episodeId] == nil else { return }
        progress[episodeId] = progress[episodeId] ?? 0
        tasks[episodeId] = Task { [weak self] in
            await self?.poll(episodeId)
        }
    }

    private func track(_ id: Int32) async {
        // Durable trigger (web: OutboxOp::TriggerDownload — survives offline
        // periods and restarts). While the op is still queued the poll below
        // simply finds no progress; the drain ships it when the server is
        // reachable and a later watch/refresh picks the run back up.
        await core.outbox?.enqueue(.triggerDownload(episodeId: id))
        await poll(id)
    }

    /// Consecutive "no tracker entry AND still NotDownloaded" polls tolerated —
    /// covers the window where the TriggerDownload op is still draining and the
    /// server hasn't registered the run (the first-tap race).
    private static let startupGracePolls = 20  // ≈30s at 1.5s steps

    private func poll(_ id: Int32) async {
        defer { tasks[id] = nil }
        var notStarted = 0
        loop: for _ in 0..<400 {  // ~10 min ceiling at 1.5s steps
            if Task.isCancelled { return }
            try? await Task.sleep(for: .seconds(1.5))
            if let snapshot = try? await core.downloadProgress(episodeId: id) {
                notStarted = 0
                if let percent = snapshot.percent {
                    progress[id] = Double(percent)
                } else if let total = snapshot.total_bytes, total > 0 {
                    progress[id] = Double(snapshot.bytes_downloaded) / Double(total)
                }
                continue
            }
            // No tracker entry: finished, failed, OR not started yet — the
            // episode's status disambiguates. A transport error just retries.
            guard let fresh = try? await core.episodeDetail(id: id) else { continue }
            switch fresh.download_status {
            case .downloading:
                notStarted = 0  // running; the tracker entry will (re)appear
            case .notDownloaded:
                notStarted += 1
                if notStarted >= Self.startupGracePolls { break loop }
            default:
                break loop  // downloaded or terminal — confirm below
            }
        }
        progress[id] = nil
        if let fresh = try? await core.episodeDetail(id: id) {
            if fresh.download_status == .downloaded {
                completed.insert(id)
                removed.remove(id)
            } else {
                failed.insert(id)
                DeviceLog.warn(
                    "server-download \(id): ended \(fresh.download_status.rawValue)")
            }
        } else {
            // Outcome unknown (offline at the final check). NO terminal state
            // here cleared the ring and left watchers spinning forever — mark
            // failed so the row is retryable; the next refresh restores truth.
            failed.insert(id)
            DeviceLog.warn("server-download \(id): outcome unknown — detail fetch failed")
        }
    }
}
