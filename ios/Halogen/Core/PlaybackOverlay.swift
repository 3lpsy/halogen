import Foundation
import Observation

/// One locally-known playback row (cursor + played flag + when we wrote it).
struct LocalPlayback: Codable {
    var cursor: UInt64
    var completed: Bool
    var updatedAt: Date
}

/// Locally-authoritative playback state keyed by episode id (the web's
/// optimistic `playbacks` overlay). Mutations commit to the shared journal and
/// cache before updating these entries.
@MainActor
@Observable
final class PlaybackOverlayModel {
    private let accountStore: LocalStore?

    private unowned let core: HalogenCore

    private(set) var entries: [Int32: LocalPlayback] = [:]

    init(core: HalogenCore) {
        self.core = core
        self.accountStore = core.store
    }

    /// Hydrate from disk (boot / account switch) so offline listening
    /// progress survives relaunch (web: hydrate_from_store → playbacks).
    func load() async {
        guard let store = accountStore else { return }
        let saved = await store.loadPlaybacks() ?? [:]
        // Keep newer in-memory entries — a save can race the hydrate.
        entries.merge(saved) { mem, disk in mem.updatedAt >= disk.updatedAt ? mem : disk }

    }

    // MARK: - mutations (optimistic + outbox, the web's command pattern)

    /// Cursor save: durable intent, followed by the visible playback overlay.
    /// History learns about the episode too — one you merely STARTED must
    /// appear there immediately, offline included (web: every
    /// set_playback_locally republishes and History recomputes from it).
    func setCursor(_ episode: EpisodeData, cursor: UInt64) {
        core.enqueueMutation(originStore: accountStore, .setCursor(episodeId: episode.id, cursor: cursor)) { [self] in
            var entry = entries[episode.id] ?? LocalPlayback(cursor: 0, completed: false, updatedAt: .now)
            entry.cursor = cursor
            entry.updatedAt = .now
            entries[episode.id] = entry
            core.models?.history.noteLocalPlayback(episode)
        }
    }

    /// Played toggle: the cursor resets to 0 in lock-step with the server op
    /// (web: `set_playback_locally` under MarkPlayed does the same), History
    /// gains the episode, and the queued op supersedes pending cursor saves.
    func markPlayed(_ episode: EpisodeData, played: Bool) {
        core.enqueueMutation(originStore: accountStore, .setPlayed(episodeId: episode.id, played: played)) { [self] in
            entries[episode.id] = LocalPlayback(cursor: 0, completed: played, updatedAt: .now)
            if played { core.models?.history.noteLocalPlayback(episode) }
        }
    }

    func removeProjected(episodeId: Int32) { entries.removeValue(forKey: episodeId) }

    /// Drop the local overlay entry for one episode — the per-episode
    /// "Remove local data" purge (web: confirm_purge wipes the local playback
    /// row; the server copy re-syncs on the next pull).
    func purge(episodeId: Int32) {
        guard entries.removeValue(forKey: episodeId) != nil else { return }
        persist()
    }

    // MARK: - overlay-wins readers

    /// The freshest known resume cursor for a row snapshot (nil = never
    /// played anywhere we know of).
    func cursor(for episode: EpisodeData) -> UInt64? {
        guard let entry = entries[episode.id] else { return episode.playback?.cursor }
        if let server = episode.playback, server.updated_at > entry.updatedAt {
            return server.cursor
        }
        return entry.cursor
    }

    /// The freshest known played facet for a row snapshot — drives menus and
    /// toggles so they never offer the wrong action after an offline toggle.
    func status(for episode: EpisodeData) -> PlaybackStatus {
        let base = episode.playback_status ?? .unplayed
        guard let entry = entries[episode.id] else { return base }
        if let server = episode.playback, server.updated_at > entry.updatedAt {
            return base
        }
        if entry.completed { return .finished }
        return entry.cursor > 0 ? .played : .unplayed
    }

    /// Persist writes are CHAINED: unordered Tasks could write an older
    /// snapshot after a newer one (each capture races to the store actor).
    private var persistChain: Task<Void, Never>?

    private func persist() {
        let snapshot = entries
        persistChain = Task { [store = accountStore, previous = persistChain] in
            await previous?.value
            await store?.savePlaybacks(snapshot)
        }
    }
}
