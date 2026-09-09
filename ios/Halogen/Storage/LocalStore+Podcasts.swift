import Foundation

extension LocalStore {
    /// Merge fetched rows and durable unsubscribe tombstones without yielding between cache reads and writes.
    func mergePodcasts(_ items: [PodcastData], excluding excluded: Set<Int32>) throws -> [PodcastData] {
        let tombstones = excluded.union(load([Int32].self, key: CacheKey.podcastTombstones) ?? [])
        let cached = load([Lossy<PodcastData>].self, key: CacheKey.podcasts)?.compactMap(\.value) ?? []
        var rows = Dictionary(cached.map { ($0.id, $0) }, uniquingKeysWith: { _, latest in latest })
        for item in items where !tombstones.contains(item.id) { rows[item.id] = item }
        for id in tombstones { rows.removeValue(forKey: id) }
        let merged = rows.values.sorted {
            let comparison = $0.title.localizedCaseInsensitiveCompare($1.title)
            return comparison == .orderedSame ? $0.id < $1.id : comparison == .orderedAscending
        }
        try saveDurably(merged, key: CacheKey.podcasts)
        return merged
    }
}
