import Foundation

/// The signed-in account and its storage namespace (web `namespace` parity):
/// embedded `e{userId}`, remote `u{userId}-{serverHash}` — user ids are
/// per-server, so the suffix keeps equal ids from sharing a cache namespace.
struct AccountContext: Equatable {
    enum Kind: Equatable {
        case embedded
        case remote(serverUrl: String)
    }

    let kind: Kind
    let userId: Int32
    let username: String

    var namespace: String {
        switch kind {
        case .embedded:
            return "e\(userId)"
        case .remote(let serverUrl):
            // 16-hex stable server hash, like the web's `u{id}-{server:016x}`.
            // (FNV-1a here vs the web's hasher — namespaces never cross the
            // device boundary, only stability within this app matters.)
            return "u\(userId)-\(Self.fnv1a16hex(serverUrl))"
        }
    }

    /// Build a context from a login: the user id comes from the API JWT's
    /// `sub` claim. Unverified decode by design — the token came from a login
    /// WE performed, and this only names a cache directory.
    static func from(kind: Kind, username: String, jwt: String) -> AccountContext? {
        guard let sub = jwtSub(jwt), let id = Int32(sub) else { return nil }
        return AccountContext(kind: kind, userId: id, username: username)
    }

    private static func fnv1a16hex(_ s: String) -> String {
        var hash: UInt64 = 0xcbf2_9ce4_8422_2325
        for byte in s.utf8 {
            hash ^= UInt64(byte)
            hash = hash &* 0x0000_0100_0000_01B3
        }
        return String(format: "%016llx", hash)
    }

    static func jwtSub(_ jwt: String) -> String? {
        let parts = jwt.split(separator: ".")
        guard parts.count == 3 else { return nil }
        var b64 = String(parts[1])
            .replacingOccurrences(of: "-", with: "+")
            .replacingOccurrences(of: "_", with: "/")
        while b64.count % 4 != 0 { b64 += "=" }
        guard let data = Data(base64Encoded: b64),
            let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else { return nil }
        if let s = obj["sub"] as? String { return s }
        if let n = obj["sub"] as? Int { return String(n) }
        return nil
    }
}
