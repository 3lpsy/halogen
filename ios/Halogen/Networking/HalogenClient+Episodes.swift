import Foundation

/// Episode download actions (server-side files).
extension HalogenClient {
    /// Ask the server to download an episode's audio from its origin.
    func triggerDownload(episodeId: Int32) async throws {
        try await postEmpty("episodes/\(episodeId)/download", body: DefaultDataType())
    }

    /// Delete the server's stored file for an episode.
    func removeServerDownload(episodeId: Int32) async throws {
        try await delete("episodes/\(episodeId)/download")
    }

    /// In-flight server download progress; nil once the tracker drops the
    /// entry (terminal — the episode row's status carries the outcome).
    func downloadProgress(episodeId: Int32) async throws -> DownloadProgressData? {
        do {
            let data: DownloadProgressData = try await get(
                "episodes/\(episodeId)/download-progress")
            return data
        } catch let error as ClientError {
            if case .http(404) = error { return nil }
            if case .api = error { return nil }
            throw error
        }
    }
}
