import Foundation

extension HalogenClient {
    func discoverPodcastPage(query: String, providers: [DiscoverProvider], cursor: String?) async throws
        -> DiscoverPodcastPageData
    {
        try await get("discover/search/page", query: discoverQuery(query, providers: providers, cursor: cursor))
    }

    func discoverEpisodePage(query: String, providers: [DiscoverProvider], cursor: String?) async throws
        -> DiscoverEpisodePageData
    {
        try await get(
            "discover/episodes/search/page", query: discoverQuery(query, providers: providers, cursor: cursor))
    }

    private func discoverQuery(_ query: String, providers: [DiscoverProvider], cursor: String?) -> [URLQueryItem] {
        var items = [URLQueryItem(name: "q", value: query)]
        for (index, provider) in providers.enumerated() {
            items.append(URLQueryItem(name: "providers[\(index)]", value: provider.rawValue))
        }
        if let cursor { items.append(URLQueryItem(name: "cursor", value: cursor)) }
        return items
    }

    func discoverPodcast(feedURL: String, provider: DiscoverProvider) async throws -> DiscoverPodcastData {
        try await get(
            "discover/podcast",
            query: [
                URLQueryItem(name: "feed_url", value: feedURL),
                URLQueryItem(name: "provider", value: provider.rawValue),
            ])
    }
}
