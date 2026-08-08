import Foundation

/// Per-account local cache — native counterpart of the web's IndexedDB
/// `LocalStore` (stale-while-revalidate). One JSON file per key under
/// `Application Support/halogen-client/<namespace>/<key>.json`, namespaced
/// per account (AccountContext) — accounts never see each other's cache.
actor LocalStore {
    private let dir: URL

    init(namespace: String) throws {
        let support = try FileManager.default.url(
            for: .applicationSupportDirectory,
            in: .userDomainMask,
            appropriateFor: nil,
            create: true
        )
        dir = support
            .appendingPathComponent("halogen-client", isDirectory: true)
            .appendingPathComponent(namespace, isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
    }

    func load<T: Codable>(_ type: T.Type, key: String) -> T? {
        guard let data = try? Data(contentsOf: fileURL(key)) else { return nil }
        return try? WireJSON.decoder.decode(T.self, from: data)
    }

    func save<T: Codable>(_ value: T, key: String) {
        guard let data = try? WireJSON.encoder.encode(value) else { return }
        try? data.write(to: fileURL(key), options: .atomic)
    }

    func remove(key: String) {
        try? FileManager.default.removeItem(at: fileURL(key))
    }

    /// Every stored key (file names sans .json) — the purge screen's census.
    func listKeys() -> [String] {
        ((try? FileManager.default.contentsOfDirectory(atPath: dir.path)) ?? [])
            .filter { $0.hasSuffix(".json") }
            .map { String($0.dropLast(5)) }
    }

    /// Total bytes across a set of keys.
    func bytes(forKeys keys: [String]) -> UInt64 {
        keys.reduce(0) { sum, key in
            let attrs = try? FileManager.default.attributesOfItem(atPath: fileURL(key).path)
            return sum + ((attrs?[.size] as? UInt64) ?? 0)
        }
    }

    func remove(keys: [String]) {
        for key in keys {
            try? FileManager.default.removeItem(at: fileURL(key))
        }
    }

    /// Wipe the whole namespace (account sign-out / cache purge).
    func wipe() {
        try? FileManager.default.removeItem(at: dir)
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
    }

    private func fileURL(_ key: String) -> URL {
        dir.appendingPathComponent("\(key).json")
    }
}

/// Element-tolerant array decoding: `[Lossy<T>]` decodes every element
/// independently, so one undecodable element (schema drift across builds)
/// yields a nil instead of failing the whole array. Callers compactMap.
struct Lossy<T: Codable>: Codable {
    let value: T?

    init(from decoder: Decoder) throws {
        value = try? T(from: decoder)
    }

    func encode(to encoder: Encoder) throws {
        try value.encode(to: encoder)
    }
}

/// The cache-key vocabulary — one flat, stable list (the native analog of the
/// web's store-key constants in ui-platform). Add entries as features land;
/// never reuse a retired key for a different shape.
enum CacheKey {
    static let podcasts = "podcasts"
    /// Unsubscribe tombstones: podcast ids locally removed whose delete op
    /// may still be queued (fetched pages filter against these).
    static let podcastTombstones = "podcast-tombstones"
    /// One snapshot per chip-set (sorted tokens keep the key stable across
    /// Set ordering); no chips = the canonical "all" snapshot.
    static func latest(_ filters: Set<EpisodeFilter>) -> String {
        filters.isEmpty
            ? "latest-all"
            : "latest-\(filters.map(\.rawValue).sorted().joined(separator: "+"))"
    }
    static let latestScrollAnchor = "latest-scroll-anchor"
    static let queueMeta = "queue-meta"
    static let playlists = "playlists"
    static func playlistEpisodes(_ id: Int32) -> String { "playlist-episodes-\(id)" }
    static func downloads(_ facet: String) -> String { "downloads-\(facet)" }
    static func episode(_ id: Int32) -> String { "episode-\(id)" }
    /// One podcast's episode list (EpisodesView's snapshot).
    static func podcastEpisodes(_ id: Int32) -> String { "podcast-episodes-\(id)" }
    /// One podcast's auto-playlist selection (ids + insert-position).
    static func autoPlaylists(_ podcastId: Int32) -> String { "auto-playlists-\(podcastId)" }
    /// A metadata (key→value) screen's rendered rows, by endpoint path.
    static func metadata(_ path: String) -> String {
        "metadata-\(path.replacingOccurrences(of: "/", with: "-"))"
    }
    static let history = "history"
    static let deviceDownloads = "device-downloads"
    /// The optimistic playback overlay (PlaybackOverlayModel) — locally-known
    /// cursors/played flags that outrank stale server rows until synced.
    static let playbacks = "playbacks-overlay"
    /// The user-forced offline toggle, re-applied at boot (web
    /// ClientConfig.manual_offline).
    static let manualOffline = "manual-offline"
}
