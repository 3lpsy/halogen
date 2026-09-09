import XCTest

@testable import Halogen

final class OutboxEncodingTests: XCTestCase {
    func testCursorRoundTripPreservesIdentityAndPosition() throws {
        let operation = OutboxOp(.setCursor(episodeId: 42, cursor: 4_294_967_296))
        let data = try JSONEncoder().encode(operation)
        let restored = try JSONDecoder().decode(OutboxOp.self, from: data)
        XCTAssertEqual(restored.id, operation.id)
        guard case .setCursor(let episodeId, let cursor) = restored.kind else {
            return XCTFail("cursor operation changed kind")
        }
        XCTAssertEqual(episodeId, 42)
        XCTAssertEqual(cursor, 4_294_967_296)
        let shared = try restored.sharedOperation()
        XCTAssertEqual(shared.id, operation.id.uuidString)
        let payload = try XCTUnwrap(
            JSONSerialization.jsonObject(with: Data(shared.operationJson.utf8)) as? [String: [String: NSNumber]])
        XCTAssertEqual(payload["SetCursor"]?["cursor"]?.uint64Value, 4_294_967_296)
    }

    func testQueueInsertionPreservesExplicitFrontPosition() throws {
        let operation = OutboxOp(.addToPlaylist(playlistId: 7, episodeId: 42, position: 0))
        let restored = try JSONDecoder().decode(
            OutboxOp.self, from: JSONEncoder().encode(operation))
        guard case .addToPlaylist(let playlistId, let episodeId, let position) = restored.kind else {
            return XCTFail("playlist operation changed kind")
        }
        XCTAssertEqual(playlistId, 7)
        XCTAssertEqual(episodeId, 42)
        XCTAssertEqual(position, 0)
        let shared = try restored.sharedOperation()
        let payload = try XCTUnwrap(
            JSONSerialization.jsonObject(with: Data(shared.operationJson.utf8)) as? [String: [String: Any]])
        XCTAssertEqual(payload["AddToPlaylist"]?["episode_ids"] as? [Int], [42])
        XCTAssertEqual(payload["AddToPlaylist"]?["position"] as? Int, 0)
    }
    func testSharedPlaylistUpdatePreservesDownloadRemovalFlags() throws {
        let operation = OutboxOp(
            .updatePlaylist(
                playlistId: 7, name: nil, isDefault: nil,
                description: nil, deleteServerFile: true, deleteClientFile: false))
        let shared = try operation.sharedOperation()
        let payload = try XCTUnwrap(
            JSONSerialization.jsonObject(with: Data(shared.operationJson.utf8)) as? [String: [String: Any]])
        XCTAssertEqual(payload["UpdatePlaylist"]?["on_remove_delete_file_server"] as? Bool, true)
        XCTAssertEqual(payload["UpdatePlaylist"]?["on_remove_delete_file_client"] as? Bool, false)
        XCTAssertNil(payload["UpdatePlaylist"]?["delete_server_file"])
        let persisted: [[Any]] = [[1, ["source_id": shared.id, "operation": payload]]]
        let data = try JSONSerialization.data(withJSONObject: persisted)
        let restored = try OutboxOp.restoreSharedQueue(String(decoding: data, as: UTF8.self))
        XCTAssertEqual(restored.count, 1)
        XCTAssertEqual(restored.first?.id, operation.id)
        XCTAssertEqual(try restored.first?.sharedOperation().operationJson, shared.operationJson)
    }

    func testJournalReplayDoesNotOverwriteNewerCachedPlayback() async throws {
        let store = try LocalStore(namespace: "test-journal-\(UUID().uuidString)")
        let operation = OutboxOp(.setCursor(episodeId: 42, cursor: 50))
        try await store.projectJournal([operation])
        let projected = await store.loadPlaybacks()
        XCTAssertEqual(projected?[42]?.cursor, 50)
        let newer = [Int32(42): LocalPlayback(cursor: 120, completed: false, updatedAt: .now)]
        try await store.savePlaybacksDurably(newer)
        try await store.projectJournal([operation])
        let replayed = await store.loadPlaybacks()
        XCTAssertEqual(replayed?[42]?.cursor, 120)
        let next = OutboxOp(.setPlayed(episodeId: 42, played: true))
        try await store.projectJournal([operation, next])
        let completed = await store.loadPlaybacks()
        XCTAssertEqual(completed?[42]?.cursor, 0)
        XCTAssertEqual(completed?[42]?.completed, true)
        try await store.projectJournal([OutboxOp(.setCursor(episodeId: 42, cursor: 10))])
        let restarted = await store.loadPlaybacks()
        XCTAssertEqual(restarted?[42]?.completed, false)
        XCTAssertEqual(restarted?[42]?.cursor, 10)
        await store.wipe()
    }

    func testLegacyPlaybackCachePreservesOtherEpisodesDuringReplay() async throws {
        let store = try LocalStore(namespace: "test-legacy-playback-\(UUID().uuidString)")
        // Swift encoded Int32-keyed dictionaries as alternating arrays in the legacy cache.
        let legacy = [Int32(41): LocalPlayback(cursor: 90, completed: false, updatedAt: .now)]
        try await store.saveDurably(legacy, key: CacheKey.playbacks)
        let restored = await store.loadPlaybacks()
        XCTAssertEqual(restored?[41]?.cursor, 90)
        try await store.projectJournal([OutboxOp(.setCursor(episodeId: 42, cursor: 50))])
        let replayed = await store.loadPlaybacks()
        XCTAssertEqual(replayed?[41]?.cursor, 90)
        XCTAssertEqual(replayed?[42]?.cursor, 50)
        await store.wipe()
    }

    func testSnapshotRecoveryPreservesChangesAfterItsCursor() async throws {
        let store = try LocalStore(namespace: "test-snapshot-\(UUID().uuidString)")
        let snapshot: [String: Any] = [
            "reset_cache": true, "sync_cursor": "cursor-one",
            "podcasts": [], "episodes": [], "playlists": [], "playbacks": [], "auto_playlists": [:],
        ]
        let json = String(decoding: try JSONSerialization.data(withJSONObject: snapshot), as: UTF8.self)
        let first = try await store.projectSnapshot(json)
        XCTAssertTrue(first)
        let operation = OutboxOp(.setCursor(episodeId: 42, cursor: 75))
        try await store.projectJournal([operation])
        let repeated = try await store.projectSnapshot(json)
        XCTAssertFalse(repeated)
        let saved = await store.loadPlaybacks()
        XCTAssertEqual(saved?[42]?.cursor, 75)
        // Missing cursor publication models an interrupted multi-file snapshot write.
        await store.remove(key: "sync-projected-cursor")
        try await store.projectSnapshot(json)
        try await store.projectJournal([operation])
        let recovered = await store.loadPlaybacks()
        XCTAssertEqual(recovered?[42]?.cursor, 75)
        await store.wipe()
    }

    func testBackgroundSyncNotifiesAfterDurableSnapshotChanges() async throws {
        let store = try LocalStore(namespace: "test-sync-notification-\(UUID().uuidString)")
        let snapshot: [String: Any] = [
            "reset_cache": true, "sync_cursor": "new-library",
            "podcasts": [], "episodes": [], "playlists": [], "playbacks": [], "auto_playlists": [:],
        ]
        let json = String(decoding: try JSONSerialization.data(withJSONObject: snapshot), as: UTF8.self)
        let outbox = try await Outbox(
            store: store,
            perform: { _ in
                SyncDrainReport(
                    applied: 0, appliedIds: [], quarantinedIds: [], quarantined: 0,
                    pending: 0, authPaused: false, lastError: nil)
            },
            synchronize: { _, _ in json },
            onSnapshot: {
                let cursor = await store.load(String.self, key: "sync-projected-cursor")
                XCTAssertEqual(cursor, "new-library")
                let count = await store.load(Int.self, key: "snapshot-notifications") ?? 0
                await store.save(count + 1, key: "snapshot-notifications")
            }
        )
        await outbox.drain()
        await outbox.drain()
        let notifications = await store.load(Int.self, key: "snapshot-notifications")
        XCTAssertEqual(notifications, 1, "Only a newly committed snapshot should invalidate the open library")
        await store.wipe()
    }

}
