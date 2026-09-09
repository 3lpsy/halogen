import UIKit

/// Authenticated artwork fetcher with a two-tier cache: NSCache in memory and
/// per-account files on disk — artwork survives offline relaunches (the
/// server's art cache is the durable origin tier). `AsyncImage` can't send
/// headers, so fetches carry the API bearer explicitly.
@MainActor
final class ArtLoader {
    static let shared = ArtLoader()

    private let cache = NSCache<NSURL, UIImage>()
    private var token: String?
    private var dir: URL?
    private var namespace: String?
    private var generation = 0

    /// Reconfigure at login/account switch: fresh token + per-account art dir.
    func configure(token: String?, namespace: String? = nil) {
        self.token = token
        if namespace == nil || namespace != self.namespace {
            generation += 1
            cache.removeAllObjects()
        }
        if let namespace { self.namespace = namespace }
        if token == nil && namespace == nil { self.namespace = nil }
        if token == nil && namespace == nil { dir = nil }
        if let namespace {
            let support = try? FileManager.default.url(
                for: .applicationSupportDirectory, in: .userDomainMask,
                appropriateFor: nil, create: true)
            dir = support?
                .appendingPathComponent("halogen-client", isDirectory: true)
                .appendingPathComponent(namespace, isDirectory: true)
                .appendingPathComponent("art", isDirectory: true)
            if let dir {
                try? FileManager.default.createDirectory(
                    at: dir, withIntermediateDirectories: true)
            }
        }
    }

    func image(for url: URL) async -> UIImage? {
        if let hit = cache.object(forKey: url as NSURL) {
            return hit
        }
        let requestGeneration = generation
        let file = diskURL(for: url)
        if let file, let data = try? Data(contentsOf: file), let image = UIImage(data: data) {
            cache.setObject(image, forKey: url as NSURL)
            return image
        }
        var request = URLRequest(url: url)
        if let token {
            request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        }
        guard let (data, response) = try? await LocalTransport.data(for: request),
            (response as? HTTPURLResponse).map({ (200..<300).contains($0.statusCode) }) == true,
            let image = UIImage(data: data)
        else { return nil }
        guard requestGeneration == generation, !Task.isCancelled else { return nil }
        cache.setObject(image, forKey: url as NSURL)
        if let file {
            try? data.write(to: file, options: .atomic)
        }
        return image
    }

    /// Stable per-URL filename (path hash) inside the account's art dir.
    private func diskURL(for url: URL) -> URL? {
        guard let dir else { return nil }
        var hash: UInt64 = 0xcbf2_9ce4_8422_2325
        for byte in url.path.utf8 {
            hash ^= UInt64(byte)
            hash = hash &* 0x0000_0100_0000_01B3
        }
        return dir.appendingPathComponent(String(format: "%016llx.img", hash))
    }
}
