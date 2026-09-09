import Foundation
import Observation

/// The Downloads tab: the SERVER's download set (Downloaded / Downloading
/// facets). DIVERGENCE from web: the server set is shown for both embedded
/// and remote (the web shows the device set on remote servers).
@MainActor
@Observable
final class DownloadsModel {
    private let accountStore: LocalStore?

    enum Facet: String, CaseIterable, Identifiable {
        case onDevice = "on_device"
        case downloaded
        case downloading

        var id: String { rawValue }
        var label: String {
            switch self {
            case .onDevice: return "On device"
            case .downloaded: return "Server"
            case .downloading: return "Downloading"
            }
        }
        var token: String { rawValue.uppercased() }
    }

    private unowned let core: HalogenCore

    private(set) var episodes: [EpisodeData] = []
    private(set) var loaded = false
    private(set) var error: String?
    private(set) var hasMore = false
    /// Last loadMore failed — the sentinel renders a retry (LoadMoreRow).
    private(set) var loadMoreFailed = false
    private var page = 0
    private var loadingMore = false
    var facet: Facet = .onDevice
    /// Search + order (facet fixed by the segmented control). Persisted
    /// across relaunch (web use_list_view_state("downloads")).
    var query = ListQuery() {
        didSet {
            guard query != oldValue else { return }
            let snapshot = query
            Task { [store = accountStore] in
                await store?.save(snapshot, key: "listquery-downloads")
            }
        }
    }
    private var loadedQuery = false

    /// Embedded accounts have no device tier — the server set IS this device.
    var availableFacets: [Facet] {
        core.isEmbeddedAccount ? [.downloaded, .downloading] : Facet.allCases
    }

    init(core: HalogenCore) {
        self.core = core
        self.accountStore = core.store
        if core.isEmbeddedAccount {
            facet = .downloaded
        }
    }

    /// Unsubscribe cascade: drop the podcast's rows from the visible list
    /// (the snapshots are pruned by the core's purge).
    func removePodcastLocally(_ podcastId: Int32) {
        episodes.removeAll { $0.podcast_id == podcastId }
    }

    /// The facet whose rows currently populate `episodes` — a facet switch
    /// must repaint from ITS cache, but a mere query keystroke must not.
    private var paintedFacet: Facet?

    func load() async {
        error = nil
        if !loadedQuery {
            loadedQuery = true
            if let store = accountStore,
                let saved = await store.load(ListQuery.self, key: "listquery-downloads")
            {
                query = saved
            }
        }
        // Cache-paint only a COLD list or a facet switch (siblings' rule):
        // this refires on every query keystroke, and repainting the full
        // unfiltered snapshot over live search results flashes the whole
        // list per key. `loaded` stays as-is (no empty-state blink).
        let facetChanged = paintedFacet != facet
        if episodes.isEmpty || facetChanged, query.search.isEmpty {
            if facetChanged { episodes = [] }
            if let store = accountStore,
                let cached = await store.load(
                    [EpisodeData].self, key: CacheKey.downloads(facet.rawValue))
            {
                episodes = cached
                loaded = true
            }
        }
        paintedFacet = facet
        await refresh()
    }

    private var generation = 0

    func refresh() async {
        if !query.search.isEmpty {
            do { try await Task.sleep(for: .milliseconds(300)) } catch { return }
        }
        if facet == .onDevice {
            // The device set is local — query applies in memory, fully offline.
            let all = core.models?.device.onDevice ?? []
            episodes = query.apply(
                to: all,
                status: { [weak core] in core?.overlayStatus($0) ?? $0.playback_status ?? .unplayed })
            error = nil
            loaded = true
            return
        }
        do {
            let first = try await core.forAccount(accountStore).latestEpisodesRaw(
                extra: [URLQueryItem(name: "filter[download_status]", value: facet.token)]
                    + query.queryItems,
                page: 0, pageSize: 20)
            episodes = first.items
            hasMore = first.hasMore
            page = 0
            error = nil
            if query.search.isEmpty {
                // Prefix-merge like the other list snapshots: fresh page 0
                // leads, the cached tail survives so scrolled-through rows
                // stay renderable offline.
                let ids = Set(first.items.map(\.id))
                var snapshot = first.items
                if let prior = await accountStore?.load(
                    [EpisodeData].self, key: CacheKey.downloads(facet.rawValue))
                {
                    snapshot += prior.filter { !ids.contains($0.id) }
                }
                await accountStore?.save(snapshot, key: CacheKey.downloads(facet.rawValue))
            }
        } catch {
            if episodes.isEmpty { self.error = FriendlyError.message(error) }
        }
        loaded = true
    }

    func loadMore() async {
        guard facet != .onDevice, hasMore, !loadingMore, loaded else { return }
        loadingMore = true
        defer { loadingMore = false }
        loadMoreFailed = false
        do {
            let next = try await core.forAccount(accountStore).latestEpisodesRaw(
                extra: [URLQueryItem(name: "filter[download_status]", value: facet.token)]
                    + query.queryItems,
                page: page + 1, pageSize: 20)
            page += 1
            let known = Set(episodes.map(\.id))
            episodes.append(contentsOf: next.items.filter { !known.contains($0.id) })
            hasMore = next.hasMore
        } catch {
            loadMoreFailed = true
        }
    }

    /// On-device facet: delete the local copy. Server facets: delete the
    /// server file — durable (web: RemoveServerDownload op), so it queues
    /// offline instead of silently failing.
    func removeDownload(_ episode: EpisodeData) async {
        if facet == .onDevice {
            episodes.removeAll { $0.id == episode.id }
            core.models?.device.remove(episode.id)
            return
        }
        guard await core.ensureQueued(originStore: accountStore, .removeServerDownload(episodeId: episode.id)) else {
            return
        }
        episodes.removeAll { $0.id == episode.id }
        // Optimistic overlay (every row/menu flips immediately) AND snapshot
        // patch — load()'s cache-paint must not resurrect the removed row.
        core.models?.serverDownloads.markRemovedLocally(episode.id)
        let snapshot = episodes
        await accountStore?.save(snapshot, key: CacheKey.downloads(facet.rawValue))
    }
}
