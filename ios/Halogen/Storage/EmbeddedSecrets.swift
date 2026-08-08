import Foundation
import Security

/// App-side mirror of the embedded server's silent-login secrets (username →
/// app-generated password). One Keychain item, independent of the session
/// registry, so removing an account never destroys its only credential copy.
enum EmbeddedSecrets {
    private static let service = "org.fgsec.halogen.embedded-secrets"
    private static let account = "users"

    static func password(for username: String) -> String? {
        loadMap()[username]
    }

    static func has(_ username: String) -> Bool {
        loadMap()[username] != nil
    }

    static func remember(username: String, password: String) {
        var map = loadMap()
        map[username] = password
        save(map)
    }

    /// Local Data → destroy embedded library: the server-side users are gone,
    /// so their credentials are dead weight.
    static func clear() {
        SecItemDelete(baseQuery() as CFDictionary)
    }

    // MARK: - keychain plumbing (same shape as SessionStore)

    private static func loadMap() -> [String: String] {
        guard let data = readItem(),
            let map = try? JSONDecoder().decode([String: String].self, from: data)
        else { return [:] }
        return map
    }

    private static func save(_ map: [String: String]) {
        guard let data = try? JSONEncoder().encode(map) else { return }
        writeItem(data)
    }

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
