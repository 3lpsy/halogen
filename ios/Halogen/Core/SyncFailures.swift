import Foundation
import Observation

/// One dead-lettered outbox op: a change the user made that the server
/// permanently rejected or that exhausted its retry budget.
struct SyncFailure: Codable, Identifiable, Equatable {
    let id: UUID
    let summary: String
    let reason: String
    let at: Date
}

/// Persisted record of dropped sync ops, plus the toast at drop time — a
/// discarded change must never be silent (the web toasts on outbox
/// dead-letter; DeviceLog alone is not a user surface). Capped ring;
/// surfaced in Settings.
@MainActor
@Observable
final class SyncFailures {
    private static let key = "sync-failures"
    private static let cap = 20

    private let store: LocalStore
    private(set) var failures: [SyncFailure] = []

    init(store: LocalStore) {
        self.store = store
        Task { failures = await store.load([SyncFailure].self, key: Self.key) ?? [] }
    }

    var count: Int { failures.count }

    func record(op: OutboxOp, error: Error) {
        let failure = SyncFailure(
            id: UUID(), summary: op.kind.summary,
            reason: FriendlyError.message(error), at: Date())
        failures = Array((failures + [failure]).suffix(Self.cap))
        ToastCenter.shared.error("Couldn't sync \"\(failure.summary)\" — \(failure.reason)")
        Task { await store.save(failures, key: Self.key) }
    }

    func clear() {
        failures = []
        Task { await store.save(failures, key: Self.key) }
    }
}

extension OutboxOp.Kind {
    /// Short human phrase for the sync-failure surfaces.
    var summary: String {
        switch self {
        case .setCursor: return "Save playback position"
        case .setPlayed(_, let played): return played ? "Mark played" : "Mark unplayed"
        case .addToPlaylist: return "Add to playlist"
        case .removeFromPlaylist: return "Remove from playlist"
        case .moveInPlaylist: return "Reorder playlist"
        case .reorderPlaylist: return "Sort playlist"
        case .updatePlaylist: return "Edit playlist"
        case .movePlaylist: return "Reorder playlists"
        case .subscribe(let feedUrl, _, _): return "Subscribe (\(feedUrl))"
        case .unsubscribe: return "Unsubscribe podcast"
        case .triggerDownload: return "Download to server"
        case .removeServerDownload: return "Remove server download"
        case .updatePodcastConfig: return "Update podcast settings"
        case .removePodcastConfig: return "Remove podcast settings"
        case .setAutoPlaylists: return "Update auto-playlists"
        }
    }
}
