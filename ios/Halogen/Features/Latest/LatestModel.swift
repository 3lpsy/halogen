import Foundation
import Observation

/// View-state for the Latest tab: local-first (cached snapshot renders before
/// the network answers), paginated append, and a persisted scroll anchor.
/// Lives OUTSIDE the view (owned by the models registry), so tab switches and
/// navigation pops come back to the exact list + scroll state.
@MainActor
@Observable
final class LatestModel {
    private let accountStore: LocalStore?

    private unowned let core: HalogenCore

    private(set) var episodes: [EpisodeData] = []
    private(set) var hasMore = false
    /// Last loadMore failed — the sentinel renders a retry (LoadMoreRow).
    private(set) var loadMoreFailed = false
    private(set) var isLoadingMore = false
    private(set) var loaded = false
    private(set) var error: String?

    /// One-shot scroll-restore target — the view scrolls to it and clears it.
    var pendingScrollTo: Int32?
    /// Search + facet + order (the persistent list-controls bar).
    var query = ListQuery()

    private var page = 0
    private var generation = 0
    private var restoredAnchor = false
    /// Row ids currently on screen (List onAppear/onDisappear) — the topmost
    /// one is the scroll anchor persisted across relaunches.
    private var visible: Set<Int32> = []

    init(core: HalogenCore) {
        self.core = core
        self.accountStore = core.store
    }

    private var loadedQuery = false

    /// First appearance / query change: cached snapshot first, then refresh.
    func load() async {
        error = nil
        if !loadedQuery {
            loadedQuery = true
            if let store = accountStore,
                let saved = await store.load(ListQuery.self, key: "listquery-latest"),
                saved != query
            {
                // A genuinely DIFFERENT restored query: adopt it and let
                // task(id: query) refire. When saved == query fall through —
                // an equal assignment never refires, and returning left the
                // tab blank until a manual refresh.
                query = saved
                return
            }
        }
        let snapshot = query
        Task { [store = accountStore] in await store?.save(snapshot, key: "listquery-latest") }
        // Debounce typing: task(id: query) cancels this sleep on the next
        // keystroke, so only the settled query actually fetches.
        if !query.search.isEmpty {
            do { try await Task.sleep(for: .milliseconds(300)) } catch { return }
        }
        // Cache-paint only a COLD list. This runs on every tab return
        // (`task(id:)` refires on appear), and the cache holds one page —
        // painting it over a deep in-memory list truncated everything past
        // page 1 and threw the scroll position away.
        if episodes.isEmpty, isDefaultish, !isDeviceSet, let store = accountStore,
            let cached = await store.load([EpisodeData].self, key: CacheKey.latest(query.filters))
        {
            episodes = cached
            loaded = true
            await restoreAnchorIfNeeded()
        }
        await refresh()
        await restoreAnchorIfNeeded()
    }

    /// Cache only canonical views (no search, default order) — one snapshot
    /// per chip-set, same as before the controls bar.
    private var isDefaultish: Bool {
        query.search.isEmpty && query.orderField == .published && query.direction == .desc
    }

    /// OnDevice chip on a non-embedded account: the list IS the local device
    /// set (the server can't express it) — rendered wholesale, no paging
    /// (web: the OnDevice branch of the AllEpisodes source).
    private var isDeviceSet: Bool {
        query.filters.contains(.onDevice) && !core.isEmbeddedAccount
    }

    /// Replace with a fresh first page and re-snapshot the cache. On failure
    /// the cached content stays on screen; the error only surfaces when
    /// there's nothing better to show.
    func refresh() async {
        generation += 1
        let mine = generation
        if isDeviceSet {
            // Fully local: filter + sort the device set in memory (the
            // remaining chips / search / order still apply — web parity).
            let device = core.models?.device
            let all = (device?.onDevice ?? []).filter {
                device?.state(of: $0.id) == .downloaded
            }
            episodes = query.apply(
                to: all,
                status: { [weak core] in core?.overlayStatus($0) ?? $0.playback_status ?? .unplayed })
            hasMore = false
            page = 0
            error = nil
            loaded = true
            return
        }
        do {
            var first = try await core.forAccount(accountStore).latestEpisodesRaw(
                extra: query.queryItems, page: 0, pageSize: 20)
            // A multi-chip facet can't ride the wire — trim the page locally
            // (OR within the facet, same rows the web's local query keeps).
            if query.needsLocalChipFilter {
                first = HalogenClient.PageOf(
                    items: first.items.filter {
                        query.matchesChips(
                            $0, status: { [weak core] in core?.overlayStatus($0) ?? $0.playback_status ?? .unplayed })
                    },
                    hasMore: first.hasMore)
            }
            // A newer query superseded this response — drop it.
            guard mine == generation else { return }
            // Deep-scroll preserving replace: when the fresh first page's ids
            // match our loaded prefix, update those rows IN PLACE and keep the
            // tail — wholesale replace yanked the scroll position on every tab
            // return. Any real change up top still replaces wholesale.
            let freshIds = first.items.map(\.id)
            if episodes.count > first.items.count,
                episodes.prefix(first.items.count).map(\.id) == freshIds
            {
                episodes.replaceSubrange(0..<first.items.count, with: first.items)
                // `page` stays where loadMore left it; hasMore from page 0 is
                // still truthful ("more than one page exists").
                hasMore = first.hasMore
            } else {
                episodes = first.items
                hasMore = first.hasMore
                page = 0
            }
            error = nil
            if isDefaultish {
                // Persist WITHOUT shrinking the snapshot back to one page:
                // fresh page 0 leads, previously cached rows keep the tail —
                // pages the user scrolled through stay renderable offline
                // (web: every fetched page is upserted into the pool).
                let ids = Set(first.items.map(\.id))
                var snapshot = first.items
                if let prior = await accountStore?.load(
                    [EpisodeData].self, key: CacheKey.latest(query.filters))
                {
                    snapshot += prior.filter { !ids.contains($0.id) }
                }
                await accountStore?.save(snapshot, key: CacheKey.latest(query.filters))
            }
        } catch is CancellationError {
            return
        } catch let error as URLError where error.code == .cancelled {
            return
        } catch {
            guard mine == generation else { return }
            if episodes.isEmpty { self.error = FriendlyError.message(error) }
        }
        loaded = true
    }

    /// Unsubscribe cascade: drop the podcast's rows from the visible list
    /// (the snapshots are pruned by the core's purge).
    func removePodcastLocally(_ podcastId: Int32) {
        episodes.removeAll { $0.podcast_id == podcastId }
    }

    /// Append the next page (infinite scroll). Id-deduped: rows can shift
    /// between the refresh and append windows.
    func loadMore() async {
        guard hasMore, !isLoadingMore, loaded, !isDeviceSet else { return }
        // Offline (manual or detected): no paging. Without this the tail
        // sentinel kept firing server fetches while "offline".
        guard !core.isOffline else { return }
        isLoadingMore = true
        defer { isLoadingMore = false }
        loadMoreFailed = false
        // Same generation guard as refresh: a query/filter change mid-fetch
        // must drop this page, not append the OLD facet's rows into the new
        // list (and corrupt page/hasMore for the wrong facet).
        let mine = generation
        do {
            let next = try await core.forAccount(accountStore).latestEpisodesRaw(
                extra: query.queryItems, page: page + 1, pageSize: 20)
            guard mine == generation else { return }
            page += 1
            let known = Set(episodes.map(\.id))
            var items = next.items.filter { !known.contains($0.id) }
            if query.needsLocalChipFilter {
                items = items.filter {
                    query.matchesChips(
                        $0, status: { [weak core] in core?.overlayStatus($0) ?? $0.playback_status ?? .unplayed })
                }
            }
            episodes.append(contentsOf: items)
            hasMore = next.hasMore
        } catch {
            loadMoreFailed = true
        }
    }

    // MARK: - visibility / scroll anchor

    func rowAppeared(_ episode: EpisodeData) {
        visible.insert(episode.id)
        // Nearing the tail triggers the next page.
        if let idx = episodes.firstIndex(where: { $0.id == episode.id }),
            idx >= episodes.count - 8
        {
            Task { await loadMore() }
        }
    }

    func rowDisappeared(_ episode: EpisodeData) {
        visible.remove(episode.id)
    }

    /// Called when the scene leaves the foreground (and on tab disappear) —
    /// the anchor survives relaunch, not just tab switches.
    func persistScrollAnchor() {
        guard let anchor = topVisibleId, let store = accountStore else { return }
        Task { await store.save(anchor, key: CacheKey.latestScrollAnchor) }
        // Snapshot the whole loaded window (capped), not just page 1: a deep
        // anchor is only restorable when the relaunch cache-paint actually
        // contains its row. `refresh()` then prefix-merges on top.
        if isDefaultish, !isDeviceSet, episodes.count > 20 {
            let window = Array(episodes.prefix(200))
            Task { await store.save(window, key: CacheKey.latest(query.filters)) }
        }
    }

    private var topVisibleId: Int32? {
        episodes.first(where: { visible.contains($0.id) })?.id
    }

    private func restoreAnchorIfNeeded() async {
        guard !restoredAnchor, !episodes.isEmpty, let store = accountStore else { return }
        restoredAnchor = true
        if let anchor = await store.load(Int32.self, key: CacheKey.latestScrollAnchor),
            episodes.contains(where: { $0.id == anchor })
        {
            pendingScrollTo = anchor
        }
    }
}
