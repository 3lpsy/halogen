import Foundation
import Observation

/// History: episodes with a playback row, ordered by playback recency across
/// the WHOLE set (web query.rs: updated_at desc, episode-id-desc tiebreak —
/// never publish date). Bodies resolve per id into a local pool, so search /
/// chips / sorts apply over everything History has loaded.
@MainActor
@Observable
final class HistoryModel {
    private let accountStore: LocalStore?

    private unowned let core: HalogenCore

    /// The displayed rows (pool → chips/search → order).
    private(set) var episodes: [EpisodeData] = []
    private(set) var loaded = false
    private(set) var error: String?
    private(set) var hasMore = false
    /// Last loadMore failed — the sentinel renders a retry (LoadMoreRow).
    private(set) var loadMoreFailed = false
    /// Search + chips + order (chips/search apply locally over the pool).
    /// Persisted across relaunch (web use_list_view_state("history")).
    var query = ListQuery(orderField: .recency) {
        didSet {
            guard query != oldValue else { return }
            let snapshot = query
            Task { [store = accountStore] in
                await store?.save(snapshot, key: "listquery-history")
            }
            rebuild()
        }
    }
    private var loadedQuery = false

    private var page = 0
    private var loadingMore = false
    private var generation = 0
    /// Resolved episode bodies by id — History's slice of the local pool.
    private var pool: [Int32: EpisodeData] = [:]
    /// Server playback recency by episode id (from the paged /playbacks).
    private var recency: [Int32: Date] = [:]

    init(core: HalogenCore) {
        self.core = core
        self.accountStore = core.store
    }

    func load() async {
        error = nil
        if !loadedQuery {
            loadedQuery = true
            if let store = accountStore,
                let saved = await store.load(ListQuery.self, key: "listquery-history")
            {
                query = saved
            }
        }
        if pool.isEmpty, let store = accountStore,
            let cached = await store.load([EpisodeData].self, key: CacheKey.history)
        {
            for episode in cached { pool[episode.id] = episode }
            rebuild()
            loaded = true
        }
        await refresh()
    }

    func refresh() async {
        // Debounce typing (task(id: query) cancels the sleep per keystroke).
        if !query.search.isEmpty {
            do { try await Task.sleep(for: .milliseconds(300)) } catch { return }
        }
        generation += 1
        let mine = generation
        do {
            let first = try await core.forAccount(accountStore).playbacksPage(page: 0)
            guard mine == generation else { return }
            merge(first.items)
            await resolveBodies(for: first.items.map(\.episode_id))
            guard mine == generation else { return }
            hasMore = first.hasMore
            page = 0
            error = nil
            rebuild()
            persistSnapshot()
        } catch is CancellationError {
            return
        } catch let error as URLError where error.code == .cancelled {
            return
        } catch {
            guard mine == generation else { return }
            rebuild()
            if episodes.isEmpty { self.error = FriendlyError.message(error) }
        }
        loaded = true
    }

    /// Infinite scroll: the next /playbacks page, merged + re-sorted over the
    /// WHOLE pool (appending without a re-sort interleaved page-shaped blocks).
    func loadMore() async {
        guard hasMore, !loadingMore, loaded else { return }
        loadingMore = true
        defer { loadingMore = false }
        loadMoreFailed = false
        do {
            let next = try await core.forAccount(accountStore).playbacksPage(page: page + 1)
            page += 1
            hasMore = next.hasMore
            merge(next.items)
            await resolveBodies(for: next.items.map(\.episode_id))
            rebuild()
            persistSnapshot()
        } catch {
            loadMoreFailed = true
        }
    }

    /// Optimistic entry from a local played-toggle: History gains the episode
    /// immediately (offline included) — the overlay's write timestamp is what
    /// ranks it first (web: the playbacks overlay drives History membership).
    func noteLocalPlayback(_ episode: EpisodeData) {
        guard pool[episode.id] == nil else {
            rebuild()
            return
        }
        pool[episode.id] = episode
        rebuild()
        persistSnapshot()
    }

    // MARK: - pool plumbing

    /// Fold one /playbacks page into the recency map. A row whose episode has
    /// a NEWER local overlay write is skipped — the server can't outrank a
    /// not-yet-drained local toggle (web: merge_history_page's pending-op
    /// guard).
    private func merge(_ playbacks: [PlaybackData]) {
        for playback in playbacks {
            if let entry = core.models?.playbacks.entries[playback.episode_id],
                entry.updatedAt > playback.updated_at
            {
                continue
            }
            let existing = recency[playback.episode_id] ?? .distantPast
            if playback.updated_at > existing {
                recency[playback.episode_id] = playback.updated_at
            }
        }
    }

    /// Fetch bodies the pool doesn't hold yet (detail fetch embeds Podcast +
    /// Playback, so rows can name their show and draw progress).
    private func resolveBodies(for ids: [Int32]) async {
        let missing = ids.filter { pool[$0] == nil }
        guard !missing.isEmpty else { return }
        let fetched = await withTaskGroup(of: EpisodeData?.self) { group in
            for id in missing {
                group.addTask { [core, accountStore] in try? await core.forAccount(accountStore).episodeDetail(id: id) }
            }
            var out: [EpisodeData] = []
            for await episode in group {
                if let episode { out.append(episode) }
            }
            return out
        }
        for episode in fetched { pool[episode.id] = episode }
    }

    /// Recompute the displayed rows: chips + search over the pool, then the
    /// order — recency (updated_at desc, id desc tiebreak; overlay-wins) for
    /// the default query, the user's explicit sort otherwise.
    private func rebuild() {
        var filterOnly = query
        filterOnly.orderField = .position  // filter without sorting
        var rows = filterOnly.apply(
            to: Array(pool.values),
            isOnDevice: { [weak core] id in
                core?.models?.device.state(of: id) == .downloaded
            },
            status: { [weak core] in core?.overlayStatus($0) ?? $0.playback_status ?? .unplayed }
        )
        if query.orderField == .recency {
            // The dedicated recency field (History's default) — an explicit
            // Published sort is a REAL choice now, not the sentinel.
            let dates = Dictionary(
                uniqueKeysWithValues: rows.map { ($0.id, recencyDate($0)) })
            rows.sort { a, b in
                let (da, db) = (dates[a.id] ?? .distantPast, dates[b.id] ?? .distantPast)
                if da != db { return query.direction == .asc ? da < db : da > db }
                return query.direction == .asc ? a.id < b.id : a.id > b.id
            }
        } else {
            rows = ListQuery(orderField: query.orderField, direction: query.direction)
                .apply(to: rows)
        }
        episodes = rows
    }

    /// Freshest known playback moment for ordering: the local overlay write
    /// outranks the server row when newer (offline listening ranks first).
    private func recencyDate(_ episode: EpisodeData) -> Date {
        let server = max(
            recency[episode.id] ?? .distantPast,
            episode.playback?.updated_at ?? .distantPast)
        if let entry = core.models?.playbacks.entries[episode.id], entry.updatedAt > server {
            return entry.updatedAt
        }
        return server
    }

    /// Persist the POOL (not the filtered view) — chips/search must not
    /// shrink what the next cold start can render. Bodies embed their
    /// playback row, so the reload can re-derive recency order offline.
    private func persistSnapshot() {
        let snapshot = Array(pool.values)
        Task { [store = accountStore] in await store?.save(snapshot, key: CacheKey.history) }
    }
}
