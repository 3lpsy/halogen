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

/// Native UI adapter over the shared Rust journal and retry engine.
actor Outbox {
    private static let key = "outbox"
    private let store: LocalStore
    private let queue: SyncQueue
    private let gate: JournalGate
    private let synchronize: (SyncQueue, String?) async throws -> String?
    private let perform: (SyncQueue) async throws -> SyncDrainReport
    private let onSnapshot: () async -> Void
    private let onDeadLetter: (OutboxOp, Error) async -> Void
    private var ops: [OutboxOp]
    private var draining = false
    private(set) var suspended = false

    init(
        store: LocalStore,
        perform: @escaping (SyncQueue) async throws -> SyncDrainReport,
        synchronize: @escaping (SyncQueue, String?) async throws -> String? = { _, _ in nil },
        onSnapshot: @escaping () async -> Void = {},
        onDeadLetter: @escaping (OutboxOp, Error) async -> Void = { _, _ in }
    ) async throws {
        self.store = store
        self.perform = perform
        self.synchronize = synchronize
        self.onSnapshot = onSnapshot
        self.onDeadLetter = onDeadLetter
        let path = await store.syncDatabasePath
        let gate = await JournalGate.forPath(path)
        self.gate = gate
        await gate.acquire()
        defer { Task { await gate.release() } }
        let original = try await store.prepareOutboxMigration()
        self.ops = try original.map { try WireJSON.decoder.decode([OutboxOp].self, from: $0) } ?? []
        self.queue = try openSyncQueue(dbPath: path)
        try await queue.importOperations(operations: ops.map { try $0.sharedOperation() })
        try await store.projectSnapshot(queue.cachedSnapshot())
        let rejected = Set(try await queue.quarantinedIds())
        for op in ops where rejected.contains(op.id.uuidString) {
            await onDeadLetter(
                op,
                NSError(
                    domain: "HalogenSync", code: 1,
                    userInfo: [NSLocalizedDescriptionKey: "Change rejected by the server"]))
        }
        let recovery = try OutboxOp.restoreSharedQueue(await queue.pending())
        try await store.projectJournal(recovery)
        let projected = Set(recovery.map { $0.id.uuidString })
        let delivered = try await queue.deliveredIds().filter { projected.contains($0) }
        try await queue.confirmCached(sourceIds: delivered)
        try await store.forgetJournalMarkers(Set(delivered))
        let pending = Set(try await queue.pendingIds())
        ops = recovery.filter { pending.contains($0.id.uuidString) }
        try await store.saveDurably(ops, key: Self.key)
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

    func clearAll() async {
        await gate.acquire()
        defer { Task { await gate.release() } }
        do {
            try await queue.clearAll()
            try await store.saveDurably(Optional<String>.none, key: "sync-projected-cursor")
            ops.removeAll()
            try await store.saveDurably(ops, key: Self.key)
        } catch { DeviceLog.error("sync clear failed: \(error)") }
    }

    @discardableResult
    func enqueue(_ kind: OutboxOp.Kind) async -> Bool { await enqueueBatch([kind]) }

    func enqueueBatch(_ kinds: [OutboxOp.Kind]) async -> Bool {
        await gate.acquire()
        defer { Task { await gate.release() } }
        guard !Task.isCancelled else { return false }
        let operations = kinds.map(OutboxOp.init)
        let previousIds = Set(ops.map { $0.id.uuidString })
        do {
            try await queue.importOperations(operations: operations.map { try $0.sharedOperation() })
            try await store.projectJournal(operations)
            let pending = Set(try await queue.pendingIds())
            ops.removeAll { previousIds.contains($0.id.uuidString) && !pending.contains($0.id.uuidString) }
            ops.append(contentsOf: operations.filter { pending.contains($0.id.uuidString) })
            // This is a derived mirror; a missing write is rebuilt from the journal on launch.
            await store.save(ops, key: Self.key)
        } catch {
            DeviceLog.error("sync persistence failed: \(error)")
            await MainActor.run { ToastCenter.shared.error("Couldn't save this change. Please try again.") }
            return false
        }
        Task {
            await Task.yield(); await drain()
        }
        return true
    }

    var pendingOperations: [OutboxOp] { ops }

    func setSuspended(_ value: Bool) { suspended = value }

    func drain() async {
        guard !suspended, !draining else { return }
        draining = true
        await gate.acquire()
        defer { draining = false; Task { await gate.release() } }
        guard !suspended else { return }
        do {
            let submitted = Set(ops.map { $0.id.uuidString })
            try await queue.importOperations(operations: ops.map { try $0.sharedOperation() })
            let report = try await perform(queue)
            let recovery = try OutboxOp.restoreSharedQueue(await queue.pending())
            try await store.projectJournal(recovery)
            let projected = Set(recovery.map { $0.id.uuidString })
            let delivered = try await queue.deliveredIds().filter { projected.contains($0) }
            try await queue.confirmCached(sourceIds: delivered)
            try await store.forgetJournalMarkers(Set(delivered))
            var snapshotChanged = false
            if !report.authPaused, !suspended {
                snapshotChanged = try await store.projectSnapshot(
                    synchronize(queue, store.load(String.self, key: "sync-projected-cursor")))
            }
            let pending = Set(try await queue.pendingIds())
            let rejected = Set(try await queue.quarantinedIds())
            let failed = ops.filter { submitted.contains($0.id.uuidString) && rejected.contains($0.id.uuidString) }
            ops.removeAll { submitted.contains($0.id.uuidString) && !pending.contains($0.id.uuidString) }
            try await store.saveDurably(ops, key: Self.key)
            if snapshotChanged { await onSnapshot() }
            for op in failed {
                await onDeadLetter(
                    op,
                    NSError(
                        domain: "HalogenSync", code: 1,
                        userInfo: [NSLocalizedDescriptionKey: report.lastError ?? "Change rejected by the server"]))
            }
        } catch { DeviceLog.warn("sync paused: \(error)") }
    }
}
