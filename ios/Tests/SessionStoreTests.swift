import Security
import XCTest

@testable import Halogen

final class SessionStoreTests: XCTestCase {
    func testSavedAccountsSurviveReloadAndTokenUpdate() throws {
        let original = try SessionStore.loadDurably()
        defer { SessionStore.save(original) }
        SessionStore.clear()
        var session = Session(kind: .remote, serverUrl: "https://example.invalid", username: "test", token: "first")
        try SessionStore.upsertActive(session)
        XCTAssertEqual(try SessionStore.loadDurably().active, session)
        session.token = "renewed"
        try SessionStore.upsertActive(session)
        let restored = try SessionStore.loadDurably()
        XCTAssertEqual(restored.sessions.count, 1)
        XCTAssertEqual(restored.active, session)
        XCTAssertEqual(
            SessionStore.writeAttributes(Data())[kSecAttrAccessible as String] as? String,
            kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly as String)
        SessionStore.clear()
        XCTAssertNil(try SessionStore.loadDurably().active)
    }
    func testSuccessfulReadMigratesLegacyProtectionWithoutReplacingCredentials() throws {
        let credentials = Data("existing-account".utf8)
        var protection = kSecAttrAccessibleWhenUnlocked as String
        let loaded = try SessionStore.readItem(
            read: { (errSecSuccess, credentials) },
            update: { attributes in
                XCTAssertNil(attributes[kSecValueData as String])
                protection = attributes[kSecAttrAccessible as String] as? String ?? ""
                return errSecSuccess
            })
        XCTAssertEqual(loaded, credentials)
        XCTAssertEqual(protection, kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly as String)
        let retained = try SessionStore.readItem(
            read: { (errSecSuccess, credentials) }, update: { _ in errSecInteractionNotAllowed })
        XCTAssertEqual(retained, credentials)
    }

}
