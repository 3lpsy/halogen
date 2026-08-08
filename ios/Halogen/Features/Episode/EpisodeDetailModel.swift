import Foundation
import Observation

/// One episode's detail state: cached per id (offline re-open), refreshed
/// with Podcast/Playback/Chapters includes; played + download actions.
@MainActor
@Observable
final class EpisodeDetailModel {
    private unowned let core: HalogenCore
    private let episodeId: Int32

    private(set) var episode: EpisodeData?
    private(set) var error: String?

    init(core: HalogenCore, episodeId: Int32) {
        self.core = core
        self.episodeId = episodeId
    }

    func load() async {
        if episode == nil, let store = core.store,
            let cached = await store.load(EpisodeData.self, key: CacheKey.episode(episodeId))
        {
            episode = cached
        }
        await refresh()
    }

    func refresh() async {
        do {
            let fresh = try await core.episodeDetail(id: episodeId)
            episode = fresh
            error = nil
            await core.store?.save(fresh, key: CacheKey.episode(episodeId))
        } catch {
            if episode == nil { self.error = FriendlyError.message(error) }
        }
    }

    /// Toggle finished ↔ unplayed. Optimistic + outbox (works offline): the
    /// shared playbacks overlay does the enqueue and flips every other screen
    /// in lock-step; this model also patches its own cached copy.
    func togglePlayed() async {
        guard var current = episode else { return }
        let status = core.models?.playbacks.status(for: current)
            ?? current.playback_status ?? .unplayed
        let nowPlayed = status != .finished
        core.models?.playbacks.markPlayed(current, played: nowPlayed)
        current = Self.withPlaybackStatus(current, nowPlayed ? .finished : .unplayed)
        episode = current
        await core.store?.save(current, key: CacheKey.episode(episodeId))
    }

    /// Generated DTOs are immutable (let fields) — rebuild with one change.
    private static func withPlaybackStatus(
        _ e: EpisodeData, _ status: PlaybackStatus
    ) -> EpisodeData {
        EpisodeData(
            id: e.id, podcast_id: e.podcast_id, title: e.title,
            description: e.description, content_url: e.content_url, guid: e.guid,
            art_url: e.art_url, published_at: e.published_at,
            downloaded_at: e.downloaded_at, content_file_path: e.content_file_path,
            download_size: e.download_size, art_file_path: e.art_file_path,
            download_status: e.download_status,
            download_started_at: e.download_started_at,
            download_attempts: e.download_attempts, playback_status: status,
            duration_secs: e.duration_secs, created_at: e.created_at,
            updated_at: e.updated_at, podcast: e.podcast, playback: e.playback,
            chapters: e.chapters
        )
    }
}
