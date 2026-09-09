import Foundation

/// Playback cursor/played upserts, single-episode fetch, and discover search.
extension HalogenClient {
    /// Upsert the caller's playback row (resume cursor + completed flag).
    func upsertPlayback(episodeId: Int32, cursor: UInt64, completed: Bool) async throws {
        let _: PlaybackData = try await post(
            "playbacks",
            body: PlaybackStoreData(episode_id: episodeId, cursor: cursor, completed: completed)
        )
    }

    /// One page of the caller's playback rows, most-recently-updated first —
    /// the History source (web: `GET /playbacks` in load_history, updated_at
    /// desc). History derives its order from these, not from episode pages.
    func playbacks(page: Int = 0, pageSize: Int = 20) async throws -> PageOf<PlaybackData> {
        let envelope: ResponseData<[PlaybackData]> = try await getEnvelope(
            "playbacks",
            query: [
                URLQueryItem(name: "pagination[page]", value: String(page)),
                URLQueryItem(name: "pagination[size]", value: String(pageSize)),
                URLQueryItem(name: "order[direction]", value: "Desc"),
                URLQueryItem(name: "order[order_by]", value: "updated_at"),
            ])
        guard let items = envelope.data else { throw ClientError.emptyData }
        let hasMore: Bool
        if let paginator = envelope.paginator {
            hasMore = paginator.page + 1 < paginator.pages
        } else {
            hasMore = items.count == pageSize
        }
        return PageOf(items: items, hasMore: hasMore)
    }

    /// One episode with its relations (podcast / caller's playback / chapters).
    func episode(id: Int32) async throws -> EpisodeData {
        try await get(
            "episodes/\(id)",
            query: [
                URLQueryItem(name: "includes[0]", value: "Podcast"),
                URLQueryItem(name: "includes[1]", value: "Playback"),
                URLQueryItem(name: "includes[2]", value: "Chapters"),
            ])
    }

    /// Online podcast search across the server's providers (iTunes/gpodder —
    /// the server proxies; the client never talks to a third party).
    /// `providers` narrows the fan-out to the user's enabled subset
    /// (serde_qs list syntax: `providers[0]=itunes&...`); nil = all.
    func discoverSearch(query: String, providers: [DiscoverProvider]? = nil) async throws
        -> DiscoverSearchData
    {
        var items = [URLQueryItem(name: "q", value: query)]
        if let providers {
            for (idx, provider) in providers.enumerated() {
                items.append(URLQueryItem(name: "providers[\(idx)]", value: provider.rawValue))
            }
        }
        return try await get("discover/search", query: items)
    }

    /// The provider list the Discover page renders toggle chips for
    /// (`GET /discover/providers`).
    func discoverProviders() async throws -> DiscoverProvidersData {
        try await get("discover/providers")
    }

    /// Subscribe to a feed (creates the podcast; the next poll ingests it).
    func createPodcast(title: String, feedUrl: String, description: String?) async throws -> PodcastData {
        try await post(
            "podcasts",
            body: PodcastStoreData(
                title: title,
                description: description,
                feed_url: feedUrl,
                art_url: nil,
                author: nil,
                podcast_config_id: nil
            )
        )
    }

    /// The streaming audio URL (AVPlayer attaches the bearer via asset headers).
    func audioURL(episodeId: Int32) -> URL {
        base.appendingPathComponent("episodes/\(episodeId)/audio")
    }
}

/// One search provider's chip metadata (wire `DiscoverProviderInfo` — not in
/// the generated Swift set yet; same field names).
struct DiscoverProviderInfo: Codable, Equatable {
    let id: DiscoverProvider
    let label: String
    /// Whether the server can currently query this provider.
    let available: Bool
    /// Whether the toggle starts enabled when the user has no saved choice.
    let default_enabled: Bool
}

/// Response body for `GET /discover/providers`.
struct DiscoverProvidersData: Codable {
    let providers: [DiscoverProviderInfo]
}
