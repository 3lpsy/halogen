import Foundation

/// A queued offline mutation — Swift analog of the web's `OutboxOp` vocabulary,
/// Codable and persisted across relaunches. Op-for-op web mapping (single-id
/// download ops where the web batches; the endpoints are idempotent).
struct OutboxOp: Codable, Identifiable {
    enum Kind: Codable {
        // Playback
        case setCursor(episodeId: Int32, cursor: UInt64)
        case setPlayed(episodeId: Int32, played: Bool)
        // Playlist membership
        case addToPlaylist(playlistId: Int32, episodeId: Int32, position: Int32?)
        case removeFromPlaylist(playlistId: Int32, episodeId: Int32)
        case moveInPlaylist(playlistId: Int32, episodeId: Int32, to: Int32)
        // Playlist structure. `updatePlaylist` is queued only when offline —
        // online edits go direct so forms can show server errors (web rule).
        // Its later optional fields decode as nil for ops persisted before
        // they existed (same back-compat rule as the web's serde defaults).
        case reorderPlaylist(
            playlistId: Int32, field: PlaylistReorderField, direction: OrderDirection)
        case updatePlaylist(
            playlistId: Int32, name: String?, isDefault: Bool?,
            description: String?, deleteServerFile: Bool?, deleteClientFile: Bool?)
        case movePlaylist(playlistId: Int32, to: Int32)
        // Library
        case subscribe(feedUrl: String, title: String?, description: String?)
        case unsubscribe(podcastId: Int32)
        // Server downloads
        case triggerDownload(episodeId: Int32)
        case removeServerDownload(episodeId: Int32)
        // Podcast config (update/remove target existing ids — safe to drain
        // later; create stays online-only, it needs a real id back).
        case updatePodcastConfig(configId: Int32, data: PodcastConfigUpdateData)
        case removePodcastConfig(podcastId: Int32)
        // `addToStart` decodes as nil for pre-field persisted ops (web:
        // `#[serde(default)]` on SetPodcastAutoPlaylists.add_to_start).
        case setAutoPlaylists(podcastId: Int32, playlistIds: [Int32], addToStart: Bool?)
    }

    let id: UUID
    let kind: Kind

    init(_ kind: Kind) {
        self.id = UUID()
        self.kind = kind
    }
}

/// Persisted FIFO of offline mutations (models mutate optimistically, enqueue,
/// the drain syncs when reachable — web drain loop). Failure taxonomy (web
/// network.rs): permanent (4xx) dead-letters now; countable (5xx/empty) backs
/// off then dead-letters; transient (transport, 401/408/429) retries forever.
actor Outbox {
    private static let key = "outbox"
    /// Consecutive countable failures of the same head op before it is
    /// dead-lettered (web: OUTBOX_DEAD_LETTER_ATTEMPTS).
    private static let deadLetterAttempts: UInt32 = 10

    /// Drains to skip before re-attempting a head op with `failures`
    /// consecutive countable failures: 1, 3, 7, then 15 (cap) — the web's
    /// `drain_skips` backoff curve.
    private static func drainSkips(_ failures: UInt32) -> UInt32 {
        (1 << min(failures, 4)) - 1
    }

    /// Retry bookkeeping for a failing FIFO head. In-memory only — a relaunch
    /// grants a fresh budget (deterministic failures re-exhaust it quickly,
    /// anything transient deserves the fresh start; same as the web).
    private struct HeadRetry {
        let opId: UUID
        var failures: UInt32
        var skipDrains: UInt32
    }

    private let store: LocalStore
    /// The core flips this with the manual-offline toggle: ops queue but
    /// never drain while suspended.
    private(set) var suspended = false
    /// Executes one op against the API. Injected by the core (owns the client).
    private let perform: (OutboxOp) async throws -> Void
    /// Fired when an op is dead-lettered — the user must hear about a
    /// discarded change (toast + sync-failures record); DeviceLog alone is
    /// not a user surface.
    private let onDeadLetter: (OutboxOp, Error) async -> Void
    private var ops: [OutboxOp]
    private var draining = false
    private var headRetry: HeadRetry?

    init(
        store: LocalStore,
        perform: @escaping (OutboxOp) async throws -> Void,
        onDeadLetter: @escaping (OutboxOp, Error) async -> Void = { _, _ in }
    ) async {
        self.store = store
        self.perform = perform
        self.onDeadLetter = onDeadLetter
        // Per-op lossy decode: ONE op persisted by a different build (unknown
        // case / changed payload) must not wipe the whole queue — an
        // all-or-nothing [OutboxOp] decode returns nil and the next persist
        // would overwrite every survivor (web: per-op serde aliases/defaults).
        let boxed = await store.load([Lossy<OutboxOp>].self, key: Self.key) ?? []
        self.ops = boxed.compactMap(\.value)
        if ops.count < boxed.count {
            DeviceLog.warn(
                "outbox: dropped \(boxed.count - ops.count) undecodable persisted op(s); kept \(ops.count)"
            )
        }
    }

    var pendingCount: Int { ops.count }

    /// Whether any queued op still targets `playlistId`'s membership/order —
    /// refresh guards keep the local list authoritative until these drain
    /// (the web's `cache_playlists` membership-preservation invariant).
    func hasPendingOps(playlistId: Int32) -> Bool {
        ops.contains {
            switch $0.kind {
            case .addToPlaylist(let id, _, _), .removeFromPlaylist(let id, _),
                .moveInPlaylist(let id, _, _), .reorderPlaylist(let id, _, _):
                return id == playlistId
            default:
                return false
            }
        }
    }

    /// Whether an Unsubscribe for `podcastId` is still queued — the podcasts
    /// tombstone set keeps its entries until their op drains (web: the
    /// worker's `unsubscribed` set outlives the op the same way).
    func hasPendingUnsubscribe(podcastId: Int32) -> Bool {
        ops.contains {
            if case .unsubscribe(let id) = $0.kind { return id == podcastId }
            return false
        }
    }

    /// Whether any queued op still reorders the playlists themselves — the
    /// playlists-list refresh keeps the local (optimistic) order until the
    /// moves drain, so server truth can't snap a drag back.
    var hasPendingPlaylistMoves: Bool {
        ops.contains {
            if case .movePlaylist = $0.kind { return true }
            return false
        }
    }

    /// Discard every queued op (the purge screen). Irreversible.
    func clearAll() async {
        ops.removeAll()
        headRetry = nil
        await persist()
    }

    func enqueue(_ kind: OutboxOp.Kind) async {
        // Cursor writes coalesce: a fresh save supersedes any queued one for
        // the episode (last-write-wins server-side); a played-toggle writes
        // cursor 0, so it supersedes too (web coalesce_pending_cursor).
        switch kind {
        case .setCursor(let episodeId, _), .setPlayed(let episodeId, _):
            ops.removeAll {
                if case .setCursor(let e, _) = $0.kind { return e == episodeId }
                return false
            }
        default:
            break
        }
        ops.append(OutboxOp(kind))
        await persist()
        await drain()
    }

    func setSuspended(_ value: Bool) {
        suspended = value
    }

    /// Pump the queue FIFO — see the type doc for the failure taxonomy.
    /// Draining stops at the head's first transient/backing-off failure so
    /// dependent ops can never replay out of order.
    func drain() async {
        guard !suspended else { return }
        guard !draining else { return }
        draining = true
        defer { draining = false }

        // A head op backing off after countable failures skips whole drains
        // per its budget (web: HeadRetry.skip_drains). A different head means
        // the old entry is stale — drop it for a fresh budget.
        if let head = ops.first {
            if var retry = headRetry, retry.opId == head.id {
                if retry.skipDrains > 0 {
                    retry.skipDrains -= 1
                    headRetry = retry
                    return
                }
            } else {
                headRetry = nil
            }
        }

        while let op = ops.first {
            do {
                try await perform(op)
                if headRetry?.opId == op.id { headRetry = nil }
                ops.removeFirst()
                await persist()
            } catch {
                switch Self.classify(error) {
                case .permanent:
                    // The server will never accept this op — dead-letter it so
                    // it stops blocking the head of the FIFO every drain.
                    DeviceLog.warn("outbox: dropped op after API rejection: \(error)")
                    if headRetry?.opId == op.id { headRetry = nil }
                    ops.removeFirst()
                    await persist()
                    await onDeadLetter(op, error)
                case .countable:
                    // 5xx/empty could equally be a transient outage or a
                    // deterministic server bug on this payload — retry, but
                    // against a budget.
                    let attempt = (headRetry?.opId == op.id ? headRetry!.failures : 0) + 1
                    if attempt >= Self.deadLetterAttempts {
                        DeviceLog.warn(
                            "outbox: op failed every budgeted retry (\(attempt)); dropping: \(error)"
                        )
                        headRetry = nil
                        ops.removeFirst()
                        await persist()
                        await onDeadLetter(op, error)
                    } else {
                        DeviceLog.warn(
                            "outbox: op failed (attempt \(attempt)); retrying with backoff: \(error)"
                        )
                        headRetry = HeadRetry(
                            opId: op.id, failures: attempt,
                            skipDrains: Self.drainSkips(attempt))
                        return
                    }
                case .transient:
                    DeviceLog.info(
                        "outbox: drain paused (retryable \(error)), \(ops.count) pending")
                    return
                }
            }
        }
    }

    private enum FailureClass {
        case permanent
        case countable
        case transient
    }

    /// The web's `is_permanent_failure` / `is_countable_failure` taxonomy
    /// (crates/ui-toast classify.rs) over the Swift client's error shape.
    private static func classify(_ error: Error) -> FailureClass {
        switch error {
        case let client as HalogenClient.ClientError:
            switch client {
            case .api:
                // Validation: the server rejected the payload — permanent.
                return .permanent
            case .http(let status):
                switch status {
                case 401, 408, 429:
                    // Token may refresh / explicit throttle-and-retry.
                    return .transient
                case 400...499:
                    return .permanent
                case 500...:
                    return .countable
                default:
                    return .transient
                }
            case .emptyData:
                return .countable
            case .offline, .signedOut:
                // Never counts against an op — waits for reconnect/re-auth.
                return .transient
            }
        case is DecodingError:
            // Version skew: the op executed but its response no longer decodes.
            // Deterministic — burn the countable budget instead of wedging the
            // head as "offline" forever (the endpoints tolerate a replay).
            return .countable
        default:
            // URLError and friends: offline — keep the op, wait for reconnect.
            return .transient
        }
    }

    private func persist() async {
        await store.save(ops, key: Self.key)
    }
}
