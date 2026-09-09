import Foundation

extension LocalStore {
    /// JSON objects match journal projection; Int32-keyed Codable dictionaries encode as arrays.
    func loadPlaybacks() -> [Int32: LocalPlayback]? {
        guard let rows = load([String: LocalPlayback].self, key: CacheKey.playbacks) else { return nil }
        var entries: [Int32: LocalPlayback] = [:]
        for (key, value) in rows {
            guard let id = Int32(key) else { return nil }
            entries[id] = value
        }
        return entries
    }

    func savePlaybacksDurably(_ entries: [Int32: LocalPlayback]) throws {
        try saveDurably(
            Dictionary(uniqueKeysWithValues: entries.map { (String($0.key), $0.value) }),
            key: CacheKey.playbacks)
    }

    func savePlaybacks(_ entries: [Int32: LocalPlayback]) {
        try? savePlaybacksDurably(entries)
    }

    /// Normalize legacy arrays before either typed reads or journal projection, preserving other episodes.
    func normalizePlaybackCache(_ payload: Any?, key: String) throws -> Any? {
        guard key == CacheKey.playbacks, let array = payload as? [Any] else { return payload }
        let legacy = try WireJSON.decoder.decode(
            [Int32: LocalPlayback].self,
            from: JSONSerialization.data(withJSONObject: array))
        let rows = Dictionary(uniqueKeysWithValues: legacy.map { (String($0.key), $0.value) })
        return try JSONSerialization.jsonObject(with: WireJSON.encoder.encode(rows))
    }
}
