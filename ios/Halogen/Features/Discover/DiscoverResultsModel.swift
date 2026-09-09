import Foundation
import Observation

enum DiscoverSearchMode: String, CaseIterable {
    case podcast = "By Podcast"
    case episode = "By Episode"
}

struct DiscoverSearchKey: Equatable {
    let query: String
    let mode: DiscoverSearchMode
    let providers: [DiscoverProvider]
}

enum DiscoverResultsPage {
    case podcasts(DiscoverPodcastPageData)
    case episodes(DiscoverEpisodePageData)

    var page: DiscoverPageInfo {
        switch self {
        case .podcasts(let data): data.page
        case .episodes(let data): data.page
        }
    }

    var errors: [DiscoverProviderError] {
        switch self {
        case .podcasts(let data): data.errors
        case .episodes(let data): data.errors
        }
    }
}

/// Owns submitted searches and cursor retries; stale requests never publish into a newer search.
@MainActor
@Observable
final class DiscoverResultsModel {
    typealias Fetch = (DiscoverSearchKey, String?) async throws -> DiscoverResultsPage
    private let fetch: Fetch
    private let errorMessage: (Error) -> String
    private var generation = 0
    private var key: DiscoverSearchKey?
    private var cursor: String?
    private var loadedFirstPage = false
    private(set) var podcasts: [DiscoverResultItem] = []
    private(set) var episodes: [DiscoverEpisodeItem] = []
    private(set) var isLoading = false
    private(set) var hasMore = false
    private(set) var hasSearched = false
    private(set) var resultLimit: UInt32 = 0
    private(set) var error: String?
    private(set) var providerErrors: [DiscoverProviderError] = []

    init(fetch: @escaping Fetch, errorMessage: @escaping (Error) -> String) {
        self.fetch = fetch
        self.errorMessage = errorMessage
    }

    func invalidate(clear: Bool = false) {
        generation += 1
        key = nil
        cursor = nil
        loadedFirstPage = false
        isLoading = false
        hasMore = false
        if clear {
            podcasts = []
            episodes = []
            hasSearched = false
            error = nil
            providerErrors = []
        }
    }

    func search(_ key: DiscoverSearchKey) async {
        invalidate(clear: true)
        self.key = key
        hasSearched = true
        await loadMore()
    }

    func loadMore() async {
        guard let key, !isLoading, !loadedFirstPage || hasMore else { return }
        let request = generation
        let next = cursor
        isLoading = true
        error = nil
        do {
            let result = try await fetch(key, next)
            guard generation == request else { return }
            switch result {
            case .podcasts(let data):
                var seen = Set(podcasts.map(\.feed_url))
                podcasts.append(contentsOf: data.items.filter { seen.insert($0.feed_url).inserted })
            case .episodes(let data):
                var seen = Set(episodes.map(\.id))
                episodes.append(contentsOf: data.items.filter { seen.insert($0.id).inserted })
            }
            loadedFirstPage = true
            cursor = result.page.next_cursor
            hasMore = result.page.has_more
            resultLimit = result.page.result_limit
            providerErrors = result.errors
            isLoading = false
        } catch {
            guard generation == request else { return }
            self.error = errorMessage(error)
            isLoading = false
        }
    }

    func restart() async {
        guard let key else { return }
        await search(key)
    }
}
