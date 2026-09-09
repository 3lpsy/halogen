import XCTest

@testable import Halogen

@MainActor
final class LibraryChangesTests: XCTestCase {
    func testDelayedPollCompletionInvalidatesAnAlreadyLoadedLibrary() async {
        let changes = LibraryChanges()
        var requests = 0
        var delays = 0
        let task = changes.watch(
            jobId: 1,
            status: {
                requests += 1
                XCTAssertEqual(changes.revision, 0)
                return requests == 1 ? .running : .completed
            },
            pause: {
                delays += 1
                XCTAssertEqual(changes.revision, 0)
            })
        await task.value
        XCTAssertEqual(requests, 2)
        XCTAssertEqual(delays, 1)
        XCTAssertEqual(changes.revision, 1)
    }

    func testTransientFailureStillObservesPartiallyCommittedFailedJob() async {
        let changes = LibraryChanges()
        var requests = 0
        let task = changes.watch(
            jobId: 1,
            status: {
                requests += 1
                if requests == 1 { throw URLError(.notConnectedToInternet) }
                return .failed
            }, pause: {})
        await task.value
        XCTAssertEqual(requests, 2)
        XCTAssertEqual(changes.revision, 1)
    }

    func testDuplicateWatchSharesOneCompletion() async {
        let changes = LibraryChanges()
        var requests = 0
        let first = changes.watch(
            jobId: 1,
            status: {
                requests += 1
                return .completed
            }, pause: {})
        let second = changes.watch(
            jobId: 1,
            status: {
                XCTFail("Duplicate watch must use the existing request")
                return .completed
            }, pause: {})
        await first.value
        await second.value
        XCTAssertEqual(requests, 1)
        XCTAssertEqual(changes.revision, 1)
    }

    func testAccountSwitchRejectsLatePollCompletion() async {
        let changes = LibraryChanges()
        let task = changes.watch(
            jobId: 1,
            status: {
                changes.cancel()
                return .completed
            }, pause: {})
        await task.value
        XCTAssertEqual(changes.revision, 0)
        changes.invalidate()
        XCTAssertEqual(changes.revision, 1)
    }
}
