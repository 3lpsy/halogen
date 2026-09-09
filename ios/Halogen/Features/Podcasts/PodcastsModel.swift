import Foundation
import Observation

/// The Podcasts tab's sort vocabulary — the web offers exactly Title / Added
/// (webui/views podcasts/controls.rs FIELDS).
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
    private let accountStore: LocalStore?

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
    private var isRefreshing = false
    private var generation = 0

    var query = PodcastListQuery() {
        didSet {
            guard query != oldValue else { return }
            let snapshot = query
            Task { [store = accountStore] in await store?.save(snapshot, key: Self.queryKey) }
        }
    }

    init(core: HalogenCore) {
        self.core = core
        self.accountStore = core.store
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
        if let saved = await accountStore?.load(Set<Int32>.self, key: CacheKey.podcastTombstones) {
            tombstones.formUnion(saved)
        }
    }

    /// Optimistic unsubscribe entry point: tombstone + drop from pool/snapshot.
    func tombstoneProjected(id: Int32) {
        tombstones.insert(id)
        podcasts.removeAll { $0.id == id }
    }

    private func pruneDrainedTombstones() async {
        await loadTombstonesIfNeeded()
        guard !tombstones.isEmpty, let outbox = core.outbox else { return }
        var kept = Set<Int32>()
        for id in tombstones where await outbox.hasPendingUnsubscribe(podcastId: id) { kept.insert(id) }
        if kept != tombstones {
            tombstones = kept
            await accountStore?.save(kept, key: CacheKey.podcastTombstones)
        }
    }

    /// Boot hydration (web: the worker's `hydrate_from_store`): fill the pool
    /// from the offline snapshot WITHOUT a network refresh, so by-id route
    /// resolution and menus work before the tab is ever opened. `loaded`
    /// stays false — the first tab visit still runs the full load cycle.
    func seed() async {
        await loadTombstonesIfNeeded()
        guard podcasts.isEmpty, let store = accountStore,
            let cached = await store.load([PodcastData].self, key: CacheKey.podcasts)
        else { return }
        if podcasts.isEmpty {
            podcasts = cached.filter { !tombstones.contains($0.id) }
        }
    }

    /// A background sync must publish its durable library to an already-open tab.
    func reloadSnapshot() async {
        await pruneDrainedTombstones()
        guard let cached = await accountStore?.load([PodcastData].self, key: CacheKey.podcasts) else { return }
        podcasts = cached.filter { !tombstones.contains($0.id) }
        error = nil
    }

    func load() async {
        guard !loaded else { return }
        error = nil
        if !loadedQuery {
            loadedQuery = true
            if let store = accountStore,
                let saved = await store.load(PodcastListQuery.self, key: Self.queryKey)
            {
                query = saved
            }
        }
        if let store = accountStore,
            let cached = await store.load([PodcastData].self, key: CacheKey.podcasts)
        {
            await loadTombstonesIfNeeded()
            podcasts = cached.filter { !tombstones.contains($0.id) }
            loaded = true
        }
        await refresh()
    }

    func refresh() async {
        guard !isRefreshing else { return }
        isRefreshing = true
        generation += 1
        defer { isRefreshing = false }
        do {
            let first = try await core.forAccount(accountStore).podcasts(page: 0)
            // Tombstone filter at CACHE time: a page fetched while an
            // Unsubscribe op is still queued must not resurrect the podcast.
            await pruneDrainedTombstones()
            let items = first.items.filter { !tombstones.contains($0.id) }
            // A first page cannot invalidate the hydrated tail of a larger library.
            podcasts = Self.merging(items, into: podcasts)
            hasMore = first.hasMore
            page = 0
            error = nil
            await persistPage(items)
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
        Task { [store = accountStore, excluded = tombstones] in
            _ = try? await store?.mergePodcasts([podcast], excluding: excluded)
        }
    }

    /// Infinite scroll: append the next library page and extend the offline
    /// snapshot with it (web: every fetched page is upserted into the store).
    func loadMore() async {
        guard hasMore, !isLoadingMore, !isRefreshing, loaded else { return }
        isLoadingMore = true
        defer { isLoadingMore = false }
        loadMoreFailed = false
        let requestGeneration = generation
        let nextPage = page + 1
        do {
            let next = try await core.forAccount(accountStore).podcasts(page: nextPage)
            guard requestGeneration == generation else { return }
            page = nextPage
            let items = next.items.filter { !tombstones.contains($0.id) }
            podcasts = Self.merging(items, into: podcasts)
            hasMore = next.hasMore
            await persistPage(items)
        } catch {
            guard requestGeneration == generation else { return }
            loadMoreFailed = true
        }
    }

    /// Fresh rows replace their cached versions without dropping unvisited pages.
    static func merging(_ fresh: [PodcastData], into cached: [PodcastData]) -> [PodcastData] {
        var result = cached
        var indices = Dictionary(
            cached.enumerated().map { ($0.element.id, $0.offset) }, uniquingKeysWith: { _, last in last })
        for podcast in fresh {
            if let index = indices[podcast.id] {
                result[index] = podcast
            } else {
                indices[podcast.id] = result.count
                result.append(podcast)
            }
        }
        return result
    }

    private func persistPage(_ items: [PodcastData]) async {
        let requestGeneration = generation
        if let merged = try? await accountStore?.mergePodcasts(items, excluding: tombstones) {
            guard requestGeneration == generation else { return }
            podcasts = merged.filter { !tombstones.contains($0.id) }
        }
    }

}
