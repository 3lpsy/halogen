import XCTest

@testable import Halogen

@MainActor
final class PodcastListTests: XCTestCase {
    private func podcast(_ id: Int32, title: String) throws -> PodcastData {
        let json: [String: Any] = [
            "id": id, "title": title, "description": "", "feed_url": "https://example.invalid/\(id)",
            "created_at": "2026-01-01T00:00:00Z", "updated_at": "2026-01-01T00:00:00Z",
        ]
        return try WireJSON.decoder.decode(PodcastData.self, from: JSONSerialization.data(withJSONObject: json))
    }

    func testFirstPageRefreshKeepsCachedTailAndUpdatesExistingRows() throws {
        let cached = try [podcast(1, title: "Old"), podcast(2, title: "Tail")]
        let fresh = try [podcast(1, title: "Updated"), podcast(3, title: "New")]
        let merged = PodcastsModel.merging(fresh, into: cached)
        XCTAssertEqual(merged.map(\.id), [1, 2, 3])
        XCTAssertEqual(merged.map(\.title), ["Updated", "Tail", "New"])
    }

    func testOverlappingPagesUpdateWithoutDuplicatingRows() throws {
        let fresh = try [podcast(1, title: "First"), podcast(1, title: "Latest")]
        let merged = PodcastsModel.merging(fresh, into: [])
        XCTAssertEqual(merged.map(\.id), [1])
        XCTAssertEqual(merged.first?.title, "Latest")
    }
}
