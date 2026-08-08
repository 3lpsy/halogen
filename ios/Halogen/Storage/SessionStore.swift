import Foundation
import Security

/// One signed-in account — the iOS analog of a web accounts-registry entry.
/// Embedded non-admin users carry an app-managed `password` for silent
/// re-login (the add-embedded-user flow generates it, same as the web).
struct Session: Codable, Equatable, Identifiable {
    enum Kind: String, Codable {
        case embedded
        case remote
    }

    var id: UUID
    let kind: Kind
    /// Remote only — the server base URL. Embedded resolves its loopback URL
    /// at boot (the port is not stable identity).
    let serverUrl: String?
    var username: String
    /// The API JWT from the last login. Remote sessions resume with it and
    /// fall back to the landing page on expiry; embedded sessions re-login.
    var token: String
    /// Embedded non-admin users: app-managed password for silent re-login.
    var password: String?

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
    }
}

/// The device-global accounts registry (all saved sessions + which is
/// active), persisted as ONE Keychain item — tokens/passwords never live in
/// UserDefaults.
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
        guard let data = readItem() else {
            return StoredAccounts(sessions: [], activeId: nil)
        }
        if let registry = try? JSONDecoder().decode(StoredAccounts.self, from: data) {
            return registry
        }
        // Legacy single-session payload → wrap into a registry.
        if let single = try? JSONDecoder().decode(Session.self, from: data) {
            return StoredAccounts(sessions: [single], activeId: single.id)
        }
        return StoredAccounts(sessions: [], activeId: nil)
    }

    static func save(_ registry: StoredAccounts) {
        guard let data = try? JSONEncoder().encode(registry) else { return }
        writeItem(data)
    }

    /// Insert-or-replace (matching kind + server + username) and make active.
    static func upsertActive(_ session: Session) {
        var registry = load()
        registry.sessions.removeAll {
            $0.kind == session.kind && $0.serverUrl == session.serverUrl
                && $0.username == session.username
        }
        registry.sessions.append(session)
        registry.activeId = session.id
        save(registry)
    }

    static func switchTo(_ id: UUID) {
        var registry = load()
        guard registry.sessions.contains(where: { $0.id == id }) else { return }
        registry.activeId = id
        save(registry)
    }

    /// Remove a session; returns the next active session (if any).
    @discardableResult
    static func remove(_ id: UUID) -> Session? {
        var registry = load()
        registry.sessions.removeAll { $0.id == id }
        if registry.activeId == id {
            registry.activeId = registry.sessions.last?.id
        }
        save(registry)
        return registry.active
    }

    static func renameActive(to username: String) {
        var registry = load()
        guard let idx = registry.sessions.firstIndex(where: { $0.id == registry.activeId })
        else { return }
        registry.sessions[idx].username = username
        save(registry)
    }

    static func clear() {
        SecItemDelete(baseQuery() as CFDictionary)
    }

    // MARK: - keychain plumbing

    private static func readItem() -> Data? {
        var query = baseQuery()
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne
        var item: CFTypeRef?
        guard SecItemCopyMatching(query as CFDictionary, &item) == errSecSuccess else {
            return nil
        }
        return item as? Data
    }

    private static func writeItem(_ data: Data) {
        var query = baseQuery()
        let update = [kSecValueData as String: data]
        let status = SecItemUpdate(query as CFDictionary, update as CFDictionary)
        if status == errSecItemNotFound {
            query[kSecValueData as String] = data
            SecItemAdd(query as CFDictionary, nil)
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
