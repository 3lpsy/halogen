import XCTest

@testable import Halogen

@MainActor
final class HistoryTests: XCTestCase {
    func testRecentSortHonorsDirectionAndBreaksTiesByEpisodeId() async {
        let core = HalogenCore()
        let history = HistoryModel(core: core)
        for (id, timestamp) in [(Int32(1), 100.0), (2, 200.0), (3, 200.0)] {
            let date = Date(timeIntervalSince1970: timestamp)
            let playback = PlaybackData(
                id: id, user_id: 1, episode_id: id, cursor: 20, completed: false,
                created_at: date, updated_at: date)
            history.noteLocalPlayback(
                EpisodeData(
                    id: id, podcast_id: 1, title: "Episode \(id)", description: nil,
                    content_url: "https://example.invalid/episode.mp3", guid: nil, art_url: nil,
                    published_at: nil, downloaded_at: nil, content_file_path: nil, download_size: nil,
                    art_file_path: nil, download_status: .notDownloaded, download_started_at: nil,
                    download_attempts: nil, playback_status: .played, duration_secs: nil,
                    created_at: date, updated_at: date, podcast: nil, playback: playback, chapters: nil))
        }
        XCTAssertEqual(history.episodes.map(\.id), [3, 2, 1])
        history.query.direction = .asc
        XCTAssertEqual(history.episodes.map(\.id), [1, 2, 3])
        history.query.direction = .desc
        XCTAssertEqual(history.episodes.map(\.id), [3, 2, 1])
    }
}
