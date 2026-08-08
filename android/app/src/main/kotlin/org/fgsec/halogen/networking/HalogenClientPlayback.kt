package org.fgsec.halogen.networking

import kotlinx.serialization.Serializable
import kotlinx.serialization.builtins.ListSerializer
import okhttp3.HttpUrl
import org.fgsec.halogen.wire.DiscoverProvider
import org.fgsec.halogen.wire.DiscoverSearchData
import org.fgsec.halogen.wire.EpisodeData
import org.fgsec.halogen.wire.PlaybackData
import org.fgsec.halogen.wire.PlaybackStoreData
import org.fgsec.halogen.wire.PodcastData
import org.fgsec.halogen.wire.PodcastStoreData

/// One search provider's chip metadata (wire `DiscoverProviderInfo` — not in
/// the generated set yet; same field names).
@Serializable
data class DiscoverProviderInfo(
    val id: DiscoverProvider,
    val label: String,
    /// Whether the server can currently query this provider.
    val available: Boolean,
    /// Whether the toggle starts enabled when the user has no saved choice.
    val default_enabled: Boolean,
)

/// Response body for `GET /discover/providers`.
@Serializable
data class DiscoverProvidersData(val providers: List<DiscoverProviderInfo>)

// Playback cursor/played upserts, single-episode fetch, and discover search.

/// Upsert the caller's playback row (resume cursor + completed flag).
suspend fun HalogenClient.upsertPlayback(episodeId: Int, cursor: ULong, completed: Boolean) {
    post(
        "playbacks",
        PlaybackStoreData(episode_id = episodeId, cursor = cursor, completed = completed),
        PlaybackStoreData.serializer(),
        PlaybackData.serializer(),
    )
}

/// One page of the caller's playback rows, most-recently-updated first — the
/// History source (web: `GET /playbacks` in load_history, updated_at desc).
/// History derives its order from these, not from episode pages.
suspend fun HalogenClient.playbacks(
    page: Int = 0,
    pageSize: Int = 20,
): HalogenClient.PageOf<PlaybackData> {
    val envelope = getEnvelope(
        "playbacks",
        listOf(
            "pagination[page]" to page.toString(),
            "pagination[size]" to pageSize.toString(),
            "order[direction]" to "Desc",
            "order[order_by]" to "updated_at",
        ),
        ListSerializer(PlaybackData.serializer()),
    )
    return pageOf(envelope, pageSize)
}

/// One episode with its relations (podcast / caller's playback / chapters).
suspend fun HalogenClient.episode(id: Int): EpisodeData =
    get(
        "episodes/$id",
        listOf(
            "includes[0]" to "Podcast",
            "includes[1]" to "Playback",
            "includes[2]" to "Chapters",
        ),
        EpisodeData.serializer(),
    )

/// Online podcast search across the server's providers (iTunes/gpodder — the
/// server proxies; the client never talks to a third party). `providers`
/// narrows the fan-out to the user's enabled subset (serde_qs list syntax:
/// `providers[0]=itunes&...`); null = all.
suspend fun HalogenClient.discoverSearch(
    query: String,
    providers: List<DiscoverProvider>? = null,
): DiscoverSearchData {
    val items = mutableListOf("q" to query)
    providers?.forEachIndexed { idx, provider ->
        items.add("providers[$idx]" to provider.string)
    }
    return get("discover/search", items, DiscoverSearchData.serializer())
}

/// The provider list the Discover page renders toggle chips for
/// (`GET /discover/providers`).
suspend fun HalogenClient.discoverProviders(): DiscoverProvidersData =
    get("discover/providers", emptyList(), DiscoverProvidersData.serializer())

/// Subscribe to a feed (creates the podcast; the next poll ingests it).
suspend fun HalogenClient.createPodcast(
    title: String,
    feedUrl: String,
    description: String?,
): PodcastData =
    post(
        "podcasts",
        PodcastStoreData(
            title = title,
            description = description,
            feed_url = feedUrl,
            art_url = null,
            author = null,
            podcast_config_id = null,
        ),
        PodcastStoreData.serializer(),
        PodcastData.serializer(),
    )

/// The streaming audio URL (Media3 attaches the bearer via request headers).
fun HalogenClient.audioUrl(episodeId: Int): HttpUrl =
    url("episodes/$episodeId/audio")
