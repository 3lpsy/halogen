import Foundation
import Observation

/// The Queue tab. The queue IS the user's default playlist (`is_default`) —
/// never a separate concept. Local-first: cached queue renders first, and
/// mutations apply optimistically then sync via the Outbox (fully offline).
@MainActor
@Observable
final class QueueModel {
    private let accountStore: LocalStore?

    private unowned let core: HalogenCore

    private(set) var queue: PlaylistData?
    private(set) var episodes: [EpisodeData] = []
    /// Search + facet + order, applied LOCALLY (the queue is a small,
    /// position-ordered list already in memory). Persisted across relaunch
    /// (web use_list_view_state("queue")).
    var query = ListQuery(orderField: .position, direction: .asc) {
        didSet {
            guard query != oldValue else { return }
            let snapshot = query
            Task { [store = accountStore] in await store?.save(snapshot, key: "listquery-queue") }
        }
    }
    private var loadedQuery = false
    private(set) var loaded = false
    private(set) var error: String?
    /// Bumped synchronously by every optimistic mutation. A refresh captures
    /// it before its fetch and discards the response if it moved — a mutation
    /// landing mid-fetch must never be overwritten by the stale server list
    /// (web: cache_playlists applies its membership guard at CACHE time).
    private var mutationEpoch = 0

    init(core: HalogenCore) {
        self.core = core
        self.accountStore = core.store
    }

    func load() async {
        error = nil
        if !loadedQuery {
            loadedQuery = true
            if let store = accountStore,
                let saved = await store.load(ListQuery.self, key: "listquery-queue")
            {
                query = saved
            }
        }
        if let store = accountStore {
            if queue == nil, let cachedQueue = await store.load(PlaylistData.self, key: CacheKey.queueMeta) {
                queue = cachedQueue
            }
            if let queue, episodes.isEmpty,
                let cached = await store.load(
                    [EpisodeData].self, key: CacheKey.playlistEpisodes(queue.id))
            {
                episodes = cached
                loaded = true
            }
        }
        await refresh()
    }

    func refresh() async {
        // Drain BEFORE pulling (the web's hard-coded order everywhere): the
        // server must reflect optimistic queue membership before we read the
        // playlist back, or this fetch wipes a not-yet-shipped offline add.
        await core.outbox?.drain()
        do {
            let fresh = try await core.forAccount(accountStore).defaultPlaylist()
            queue = fresh
            if let fresh {
                await accountStore?.save(fresh, key: CacheKey.queueMeta)
                // Membership ops still queued for this playlist (the drain
                // couldn't ship them): the local list is AHEAD of the server —
                // keep it, don't overwrite screen/cache with stale membership
                // (web: cache_playlists' membership-preservation invariant).
                if let outbox = core.outbox, await outbox.hasPendingOps(playlistId: fresh.id) {
                    loaded = true
                    return
                }
                let epoch = mutationEpoch
                let served = try await core.forAccount(accountStore).playlistEpisodes(playlistId: fresh.id)
                // Guard AGAIN at cache time: a mutation (or its just-enqueued
                // op) that landed while the fetch was in flight outranks the
                // stale server membership.
                if epoch != mutationEpoch { loaded = true; return }
                if let outbox = core.outbox, await outbox.hasPendingOps(playlistId: fresh.id) {
                    loaded = true
                    return
                }
                episodes = served
                await accountStore?.save(episodes, key: CacheKey.playlistEpisodes(fresh.id))
            } else {
                episodes = []
                await accountStore?.remove(key: CacheKey.queueMeta)
            }
            error = nil
        } catch {
            if episodes.isEmpty && queue == nil { self.error = FriendlyError.message(error) }
        }
        loaded = true
    }

    /// "No queue yet" flow: create the default playlist (online-only — the
    /// created id anchors every later offline op). Failure toasts — setting
    /// `error` replaced the whole no-queue screen (and its Create button)
    /// with a full-page error.
    func createQueue() async {
        do {
            _ = try await core.forAccount(accountStore).createPlaylist(name: "Queue", isDefault: true)
            await refresh()
        } catch {
            ToastCenter.shared.error(
                "Couldn't create the queue — \(FriendlyError.message(error))")
        }
    }

    // MARK: - optimistic membership ops (offline-capable via Outbox)

    func remove(_ episode: EpisodeData) {
        guard let queue else { return }
        core.enqueueMutation(
            originStore: accountStore, .removeFromPlaylist(playlistId: queue.id, episodeId: episode.id)
        ) { [self] in
            mutationEpoch += 1
            episodes.removeAll { $0.id == episode.id }
            persistSnapshot()
        }
    }

    func move(fromOffsets: IndexSet, toOffset: Int) {
        guard let fromIndex = fromOffsets.first, episodes.indices.contains(fromIndex) else { return }
        let episode = episodes[fromIndex]
        var ordered = episodes
        ordered.move(fromOffsets: fromOffsets, toOffset: toOffset)
        guard let destination = ordered.firstIndex(where: { $0.id == episode.id }) else { return }
        move(episode, to: destination)
    }

    private func move(_ episode: EpisodeData, to destination: Int) {
        guard let queue else { return }
        let operation = OutboxOp.Kind.moveInPlaylist(
            playlistId: queue.id, episodeId: episode.id, to: Int32(destination))
        core.enqueueMutation(originStore: accountStore, operation) { [self] in
            guard let index = episodes.firstIndex(where: { $0.id == episode.id }) else { return }
            mutationEpoch += 1
            let moved = episodes.remove(at: index)
            episodes.insert(moved, at: min(destination, episodes.count))
            persistSnapshot()
        }
    }

    func moveToTop(_ episode: EpisodeData) { move(episode, to: 0) }

    func add(_ episode: EpisodeData) {
        guard let queue else {
            ToastCenter.shared.error("Queue isn't loaded yet. Reconnect once, then retry.")
            return
        }
        let front = core.models?.prefs.prefs.addToQueueFront ?? true
        core.enqueueMutation(
            originStore: accountStore,
            .addToPlaylist(playlistId: queue.id, episodeId: episode.id, position: front ? 0 : nil), episode: episode
        ) { [self] in
            guard !episodes.contains(where: { $0.id == episode.id }) else { return }
            mutationEpoch += 1
            if front { episodes.insert(episode, at: 0) } else { episodes.append(episode) }
            persistSnapshot()
        }
    }

    /// The rows the view renders. Reordering is only meaningful over the raw
    /// position order with no search/filter applied.
    var displayed: [EpisodeData] {
        query.apply(
            to: episodes,
            isOnDevice: { [weak core] id in core?.models?.device.state(of: id) == .downloaded },
            status: { [weak core] in core?.overlayStatus($0) ?? $0.playback_status ?? .unplayed }
        )
    }

    var reorderable: Bool {
        query == ListQuery(orderField: .position, direction: .asc)
    }

    func contains(_ episode: EpisodeData) -> Bool {
        episodes.contains(where: { $0.id == episode.id })
    }

    private func persistSnapshot() {
        guard let queue else { return }
        // Keep the playlists pool's episode_ids for the queue row in step
        // (menus and counts read it — see PlaylistsModel.setMembership).
        core.models?.playlists.setMembership(
            playlistId: queue.id, episodeIds: episodes.map(\.id))

    }
}
