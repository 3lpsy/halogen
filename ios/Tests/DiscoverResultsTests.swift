import XCTest

@testable import Halogen

@MainActor
final class DiscoverResultsTests: XCTestCase {
    @MainActor
    private final class Requests {
        var pending: [CheckedContinuation<DiscoverResultsPage, Error>] = []
        var cursors: [String?] = []
        private var arrivals: [(count: Int, expectation: XCTestExpectation)] = []

        func fetch(_ key: DiscoverSearchKey, cursor: String?) async throws -> DiscoverResultsPage {
            cursors.append(cursor)
            return try await withCheckedThrowingContinuation { continuation in
                pending.append(continuation)
                for arrival in arrivals where pending.count >= arrival.count {
                    arrival.expectation.fulfill()
                }
                arrivals.removeAll { pending.count >= $0.count }
            }
        }

        func waitFor(_ count: Int, file: StaticString = #filePath, line: UInt = #line) async {
            guard pending.count < count else { return }
            let arrival = XCTestExpectation(description: "Mock search request \(count)")
            arrivals.append((count, arrival))
            let result = await XCTWaiter.fulfillment(of: [arrival], timeout: 5)
            XCTAssertEqual(result, .completed, "Expected a mocked search request", file: file, line: line)
        }
    }

    private func key(_ query: String) -> DiscoverSearchKey {
        DiscoverSearchKey(query: query, mode: .podcast, providers: [.itunes])
    }

    private func page(_ titles: [String], cursor: String? = nil) -> DiscoverResultsPage {
        .podcasts(
            DiscoverPodcastPageData(
                items: titles.map {
                    DiscoverResultItem(
                        id: $0, provider: .itunes, title: $0,
                        feed_url: "https://example.invalid/\($0)", description: nil, author: nil)
                }, errors: [], page: DiscoverPageInfo(has_more: cursor != nil, next_cursor: cursor, result_limit: 200)))
    }

    func testOlderResponseCannotPublishOrClearCurrentLoading() async {
        let requests = Requests()
        let model = DiscoverResultsModel(fetch: requests.fetch, errorMessage: { $0.localizedDescription })
        let old = Task { await model.search(key("old")) }
        await requests.waitFor(1)
        let current = Task { await model.search(key("current")) }
        await requests.waitFor(2)
        requests.pending[0].resume(returning: page(["old"]))
        await old.value
        XCTAssertTrue(model.isLoading)
        XCTAssertTrue(model.podcasts.isEmpty)
        requests.pending[1].resume(returning: page(["current"]))
        await current.value
        XCTAssertEqual(model.podcasts.map(\.title), ["current"])
    }

    func testOlderFailureCannotReplaceNewerResults() async {
        let requests = Requests()
        let model = DiscoverResultsModel(fetch: requests.fetch, errorMessage: { $0.localizedDescription })
        let old = Task { await model.search(key("old")) }
        await requests.waitFor(1)
        let current = Task { await model.search(key("current")) }
        await requests.waitFor(2)
        requests.pending[1].resume(returning: page(["current"]))
        await current.value
        requests.pending[0].resume(throwing: URLError(.notConnectedToInternet))
        await old.value
        XCTAssertNil(model.error)
        XCTAssertEqual(model.podcasts.map(\.title), ["current"])
        XCTAssertFalse(model.isLoading)
    }

    func testPageRetryPreservesCursorAndDeduplicatesWithoutOverlappingRequests() async {
        let requests = Requests()
        let model = DiscoverResultsModel(fetch: requests.fetch, errorMessage: { $0.localizedDescription })
        let first = Task { await model.search(key("meditation")) }
        await requests.waitFor(1)
        requests.pending[0].resume(returning: page(["first"], cursor: "next"))
        await first.value
        let more = Task { await model.loadMore() }
        await requests.waitFor(2)
        await model.loadMore()
        XCTAssertEqual(requests.pending.count, 2)
        requests.pending[1].resume(throwing: URLError(.timedOut))
        await more.value
        XCTAssertEqual(model.podcasts.map(\.title), ["first"])
        let retry = Task { await model.loadMore() }
        await requests.waitFor(3)
        XCTAssertEqual(requests.cursors[1], "next")
        XCTAssertEqual(requests.cursors[2], "next")
        requests.pending[2].resume(returning: page(["first", "second"]))
        await retry.value
        XCTAssertEqual(model.podcasts.map(\.title), ["first", "second"])
        XCTAssertFalse(model.hasMore)
        await model.loadMore()
        XCTAssertEqual(requests.pending.count, 3)
    }
    func testModeChangeStartsAtFirstPageAndRejectsOlderPage() async {
        let requests = Requests()
        let model = DiscoverResultsModel(fetch: requests.fetch, errorMessage: { $0.localizedDescription })
        let first = Task { await model.search(key("meditation")) }
        await requests.waitFor(1)
        requests.pending[0].resume(returning: page(["podcast"], cursor: "next"))
        await first.value
        let oldPage = Task { await model.loadMore() }
        await requests.waitFor(2)
        model.invalidate(clear: true)
        let episodes = Task {
            await model.search(DiscoverSearchKey(query: "meditation", mode: .episode, providers: [.itunes]))
        }
        await requests.waitFor(3)
        XCTAssertNil(requests.cursors[2])
        requests.pending[1].resume(returning: page(["stale"]))
        await oldPage.value
        XCTAssertTrue(model.podcasts.isEmpty)
        XCTAssertTrue(model.isLoading)
        requests.pending[2].resume(
            returning: .episodes(
                DiscoverEpisodePageData(
                    items: [
                        DiscoverEpisodeItem(
                            id: "episode", provider: .itunes, title: "Meditation",
                            feed_url: "https://example.invalid/feed", podcast_title: "Another show", description: "",
                            guid: "episode", published_at: nil, duration_seconds: nil)
                    ],
                    errors: [], page: DiscoverPageInfo(has_more: false, next_cursor: nil, result_limit: 200))))
        await episodes.value
        XCTAssertEqual(model.episodes.map(\.podcast_title), ["Another show"])
        XCTAssertTrue(model.podcasts.isEmpty)
    }

}
