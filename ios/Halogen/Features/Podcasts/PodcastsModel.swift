import Foundation
import Observation

/// The Podcasts tab's sort vocabulary — the web offers exactly Title / Added
/// (crates/ui-views podcasts/controls.rs FIELDS).
enum PodcastSortField: String, Codable, CaseIterable {
    case title
    case added

    var label: String { self == .title ? "Title" : "Added" }
}

/// Sort + search state for the Podcasts tab, persisted across navigation and
/// relaunch (web: `use_list_view_state("podcasts", ...)`, default Title asc).
struct PodcastListQuery: Codable, Equatable {
    var search: String = ""
    var field: PodcastSortField = .title
    var direction: OrderDirection = .asc
}

/// View-state for the Podcasts tab: local-first — cached library renders
/// first, then the refresh overwrites screen + snapshot. Search + sort apply
/// client-side over the loaded pool (web `apply_podcast_search_sort`).
@MainActor
@Observable
final class PodcastsModel {
    private static let queryKey = "listquery-podcasts"

    private let core: HalogenCore

    private(set) var podcasts: [PodcastData] = []
    private(set) var loaded = false
    private(set) var error: String?
    private(set) var hasMore = false
    /// Last loadMore failed — the sentinel renders a retry (LoadMoreRow).
    private(set) var loadMoreFailed = false
    private(set) var isLoadingMore = false
    private var page = 0
    private var loadedQuery = false

    var query = PodcastListQuery() {
        didSet {
            guard query != oldValue else { return }
            let snapshot = query
            Task { [store = core.store] in await store?.save(snapshot, key: Self.queryKey) }
        }
    }

    init(core: HalogenCore) {
        self.core = core
    }

    /// The rows the view renders: title+author search, then Title/Added sort
    /// (case-insensitive title, id tiebreak — web filter.rs).
    var displayed: [PodcastData] {
        var rows = podcasts
        let q = query.search.trimmingCharacters(in: .whitespaces).lowercased()
        if !q.isEmpty {
            rows = rows.filter {
                $0.title.lowercased().contains(q)
                    || ($0.author?.lowercased().contains(q) ?? false)
            }
        }
        let asc = query.direction == .asc
        switch query.field {
        case .title:
            rows.sort {
                let (a, b) = ($0.title.lowercased(), $1.title.lowercased())
                if a != b { return asc ? a < b : a > b }
                return asc ? $0.id < $1.id : $0.id > $1.id
            }
        case .added:
            rows.sort {
                if $0.created_at != $1.created_at {
                    return asc
                        ? $0.created_at < $1.created_at : $0.created_at > $1.created_at
                }
                return asc ? $0.id < $1.id : $0.id > $1.id
            }
        }
        return rows
    }

    // MARK: - unsubscribe tombstones

    /// Web worker.rs `unsubscribed`: ids removed locally whose delete op may
    /// still be queued — fetched pages filter against this so server truth
    /// can't resurrect the podcast mid-drain. Clears once no Unsubscribe op
    /// remains queued (dead-lettered → resurrecting is correct).
    private var tombstones: Set<Int32> = []
    private var loadedTombstones = false

    private func loadTombstonesIfNeeded() async {
        guard !loadedTombstones else { return }
        loadedTombstones = true
        if let saved = await core.store?.load(Set<Int32>.self, key: CacheKey.podcastTombstones) {
            tombstones.formUnion(saved)
        }
    }

    /// Optimistic unsubscribe entry point: tombstone + drop from pool/snapshot.
    func tombstone(id: Int32) async {
        await loadTombstonesIfNeeded()
        tombstones.insert(id)
        await core.store?.save(tombstones, key: CacheKey.podcastTombstones)
        removeLocally(id: id)
    }

    private func pruneDrainedTombstones() async {
        await loadTombstonesIfNeeded()
        guard !tombstones.isEmpty, let outbox = core.outbox else { return }
        var kept = Set<Int32>()
        for id in tombstones {
            if await outbox.hasPendingUnsubscribe(podcastId: id) { kept.insert(id) }
        }
        if kept != tombstones {
            tombstones = kept
            await core.store?.save(kept, key: CacheKey.podcastTombstones)
        }
    }

    /// Boot hydration (web: the worker's `hydrate_from_store`): fill the pool
    /// from the offline snapshot WITHOUT a network refresh, so by-id route
    /// resolution and menus work before the tab is ever opened. `loaded`
    /// stays false — the first tab visit still runs the full load cycle.
    func seed() async {
        await loadTombstonesIfNeeded()
        guard podcasts.isEmpty, let store = core.store,
            let cached = await store.load([PodcastData].self, key: CacheKey.podcasts)
        else { return }
        if podcasts.isEmpty {
            podcasts = cached.filter { !tombstones.contains($0.id) }
        }
    }

    func load() async {
        error = nil
        if !loadedQuery {
            loadedQuery = true
            if let store = core.store,
                let saved = await store.load(PodcastListQuery.self, key: Self.queryKey)
            {
                query = saved
            }
        }
        if let store = core.store,
            let cached = await store.load([PodcastData].self, key: CacheKey.podcasts)
        {
            await loadTombstonesIfNeeded()
            podcasts = cached.filter { !tombstones.contains($0.id) }
            loaded = true
        }
        await refresh()
    }

    func refresh() async {
        do {
            let first = try await core.podcasts(page: 0)
            // Tombstone filter at CACHE time: a page fetched while an
            // Unsubscribe op is still queued must not resurrect the podcast.
            await pruneDrainedTombstones()
            let items = first.items.filter { !tombstones.contains($0.id) }
            podcasts = items
            hasMore = first.hasMore
            page = 0
            error = nil
            // Persist WITHOUT shrinking the snapshot back to one page: fresh
            // page 0 leads, previously cached rows keep the tail — everything
            // the user ever scrolled through stays renderable offline (web:
            // CachePodcasts upserts every fetched page into the pool).
            let ids = Set(items.map(\.id))
            var snapshot = items
            if let prior = await core.store?.load([PodcastData].self, key: CacheKey.podcasts) {
                snapshot += prior.filter { !ids.contains($0.id) && !tombstones.contains($0.id) }
            }
            await core.store?.save(snapshot, key: CacheKey.podcasts)
        } catch {
            if podcasts.isEmpty { self.error = FriendlyError.message(error) }
        }
        loaded = true
    }

    /// Deep-link fetch-through cache (web `cache_podcasts`): a by-id fetched
    /// podcast joins the in-memory pool AND the offline snapshot, so the
    /// next visit — online or offline — renders instantly.
    func upsert(_ podcast: PodcastData) {
        if let idx = podcasts.firstIndex(where: { $0.id == podcast.id }) {
            podcasts[idx] = podcast
        } else {
            podcasts.append(podcast)
        }
        Task { [store = core.store] in
            var cached = await store?.load([PodcastData].self, key: CacheKey.podcasts) ?? []
            if let idx = cached.firstIndex(where: { $0.id == podcast.id }) {
                cached[idx] = podcast
            } else {
                cached.append(podcast)
            }
            await store?.save(cached, key: CacheKey.podcasts)
        }
    }

    /// Optimistic unsubscribe: drop the podcast from screen + snapshot (the
    /// durable Unsubscribe outbox op syncs the server; web tombstones + local
    /// delete the same way). The snapshot is patched in place so the cached
    /// tail beyond the loaded window survives.
    func removeLocally(id: Int32) {
        podcasts.removeAll { $0.id == id }
        let fallback = podcasts
        Task { [store = core.store] in
            var cached = await store?.load([PodcastData].self, key: CacheKey.podcasts) ?? fallback
            cached.removeAll { $0.id == id }
            await store?.save(cached, key: CacheKey.podcasts)
        }
    }

    /// Infinite scroll: append the next library page and extend the offline
    /// snapshot with it (web: every fetched page is upserted into the store).
    func loadMore() async {
        guard hasMore, !isLoadingMore, loaded else { return }
        isLoadingMore = true
        defer { isLoadingMore = false }
        loadMoreFailed = false
        do {
            let next = try await core.podcasts(page: page + 1)
            page += 1
            let known = Set(podcasts.map(\.id))
            podcasts.append(
                contentsOf: next.items.filter {
                    !known.contains($0.id) && !tombstones.contains($0.id)
                })
            hasMore = next.hasMore
            let snapshot = podcasts
            Task { [store = core.store] in await store?.save(snapshot, key: CacheKey.podcasts) }
        } catch {
            loadMoreFailed = true
        }
    }
}
