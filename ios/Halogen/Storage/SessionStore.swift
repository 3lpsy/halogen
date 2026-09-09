import Foundation
import Security

/// A saved remote account or local profile; legacy credentials decode during migration.
struct Session: Codable, Equatable, Identifiable {
    enum Kind: String, Codable {
        case embedded
        case remote
    }

    var id: UUID
    let kind: Kind
    /// Remote only. Local profiles resolve through FFI at boot.
    let serverUrl: String?
    var username: String
    /// The remote API JWT. Local profiles clear legacy tokens on first resume.
    var token: String
    /// Compatibility field for old sessions, cleared when a local profile resumes.
    var password: String?
    var localUserId: Int32?

    init(
        kind: Kind, serverUrl: String?, username: String, token: String,
        password: String? = nil
    ) {
        self.id = UUID()
        self.kind = kind
        self.serverUrl = serverUrl
        self.username = username
        self.token = token
        self.password = password
    }

    // Tolerant decode: pre-registry sessions had no id/password.
    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decodeIfPresent(UUID.self, forKey: .id) ?? UUID()
        kind = try c.decode(Kind.self, forKey: .kind)
        serverUrl = try c.decodeIfPresent(String.self, forKey: .serverUrl)
        username = try c.decode(String.self, forKey: .username)
        token = try c.decode(String.self, forKey: .token)
        password = try c.decodeIfPresent(String.self, forKey: .password)
        localUserId = try c.decodeIfPresent(Int32.self, forKey: .localUserId)
    }
}

/// The device-global accounts registry (all saved sessions + which is
/// active), persisted as one Keychain item. Only unsigned simulators use
/// a UserDefaults fallback; device credentials always stay in Keychain.
struct StoredAccounts: Codable {
    var sessions: [Session]
    var activeId: UUID?

    var active: Session? {
        sessions.first { $0.id == activeId }
    }
}

enum SessionStore {
    private static let service = "org.fgsec.halogen.session"
    private static let account = "active"

    static func load() -> StoredAccounts {
        do { return try loadDurably() } catch {
            DeviceLog.warn("session storage read failed: \(error)")
            return StoredAccounts(sessions: [], activeId: nil)
        }
    }

    static func loadDurably() throws -> StoredAccounts {
        guard let data = try readItem() else {
            return StoredAccounts(sessions: [], activeId: nil)
        }
        if let registry = try? JSONDecoder().decode(StoredAccounts.self, from: data) {
            return registry
        }
        // Legacy single-session payload → wrap into a registry.
        if let single = try? JSONDecoder().decode(Session.self, from: data) {
            return StoredAccounts(sessions: [single], activeId: single.id)
        }
        throw CocoaError(.coderReadCorrupt)
    }

    static func save(_ registry: StoredAccounts) {
        do { try saveDurably(registry) } catch {
            DeviceLog.warn("session storage write failed: \(error)")
        }
    }

    static func saveDurably(_ registry: StoredAccounts) throws {
        try writeItem(JSONEncoder().encode(registry))
    }

    /// Insert-or-replace (matching kind + server + username) and make active.
    static func upsertActive(_ session: Session) throws {
        var registry = try loadDurably()
        registry.sessions.removeAll {
            $0.kind == session.kind && $0.serverUrl == session.serverUrl
                && $0.username == session.username
        }
        registry.sessions.append(session)
        registry.activeId = session.id
        try saveDurably(registry)
    }

    static func switchTo(_ id: UUID) {
        guard var registry = try? loadDurably() else { return }
        guard registry.sessions.contains(where: { $0.id == id }) else { return }
        registry.activeId = id
        save(registry)
    }

    /// Remove a session; returns the next active session (if any).
    @discardableResult
    static func remove(_ id: UUID) -> Session? {
        guard var registry = try? loadDurably() else { return nil }
        registry.sessions.removeAll { $0.id == id }
        if registry.activeId == id {
            registry.activeId = registry.sessions.last?.id
        }
        save(registry)
        return registry.active
    }

    static func renameActive(to username: String) {
        guard var registry = try? loadDurably() else { return }
        guard let idx = registry.sessions.firstIndex(where: { $0.id == registry.activeId })
        else { return }
        registry.sessions[idx].username = username
        save(registry)
    }

    static func clear() {
        SecItemDelete(baseQuery() as CFDictionary)
        #if targetEnvironment(simulator)
            UserDefaults.standard.removeObject(forKey: simulatorFallbackKey)
        #endif
    }

    static let accessibility = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
    private static let simulatorFallbackKey = "halogen_simulator_accounts"

    static func writeAttributes(_ data: Data) -> [String: Any] {
        [kSecValueData as String: data, kSecAttrAccessible as String: accessibility]
    }

    static func readItem(
        read: () -> (OSStatus, Data?) = readKeychain,
        update: ([String: Any]) -> OSStatus = updateKeychain
    ) throws -> Data? {
        let (status, data) = read()
        if status == errSecSuccess {
            // Existing accounts migrate at first readable boot, without waiting for token renewal.
            let migration = update([kSecAttrAccessible as String: accessibility])
            if migration != errSecSuccess {
                DeviceLog.warn("session protection update failed: keychain status \(migration)")
            }
            return data
        }
        #if targetEnvironment(simulator)
            if let data = UserDefaults.standard.data(forKey: simulatorFallbackKey) { return data }
            if status == errSecMissingEntitlement { return nil }
        #endif
        if status == errSecItemNotFound { return nil }
        throw StorageError.keychain(status)
    }

    private static func readKeychain() -> (OSStatus, Data?) {
        var query = baseQuery()
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne
        var item: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &item)
        return (status, item as? Data)
    }

    private static func updateKeychain(_ attributes: [String: Any]) -> OSStatus {
        SecItemUpdate(baseQuery() as CFDictionary, attributes as CFDictionary)
    }

    private static func writeItem(_ data: Data) throws {
        let attributes = writeAttributes(data)
        var status = SecItemUpdate(baseQuery() as CFDictionary, attributes as CFDictionary)
        if status == errSecItemNotFound {
            let query = baseQuery().merging(attributes) { _, new in new }
            status = SecItemAdd(query as CFDictionary, nil)
        }
        #if targetEnvironment(simulator)
            // Unsigned simulator builds cannot use Keychain; device credentials never use this fallback.
            if status != errSecSuccess {
                DeviceLog.info("simulator session storage fallback: keychain status \(status)")
                UserDefaults.standard.set(data, forKey: simulatorFallbackKey)
                return
            }
            UserDefaults.standard.removeObject(forKey: simulatorFallbackKey)
        #endif
        guard status == errSecSuccess else { throw StorageError.keychain(status) }
    }

    enum StorageError: LocalizedError {
        case keychain(OSStatus)

        var errorDescription: String? {
            guard case .keychain(let status) = self else { return nil }
            return "Saved accounts are unavailable (Keychain \(status)). Unlock the device and try again."
        }
    }

    private static func baseQuery() -> [String: Any] {
        [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]
    }
}
