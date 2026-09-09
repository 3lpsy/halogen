import Foundation

/// Routes local application requests through the profile's Rust session.
@MainActor
enum LocalTransport {
    private static var sessions: [String: LocalCore] = [:]

    static func install(_ core: LocalCore) -> String {
        let identity = UUID().uuidString.lowercased()
        sessions[identity] = core
        return "halogen-local://\(identity)"
    }

    static func remove(base: URL?) {
        guard let base, base.scheme == "halogen-local", let host = base.host else { return }
        sessions.removeValue(forKey: host)
    }

    static func core(for url: URL) throws -> LocalCore {
        guard url.scheme == "halogen-local", let host = url.host, let core = sessions[host] else {
            throw HalogenClient.ClientError.signedOut
        }
        return core
    }

    static func data(for request: URLRequest) async throws -> (Data, URLResponse) {
        guard let url = request.url else { throw URLError(.badURL) }
        guard url.scheme == "halogen-local" else {
            return try await URLSession.shared.data(for: request)
        }
        let core = try core(for: url)
        let data: Data
        let status: Int
        let parts = url.path.split(separator: "/")
        if url.path == "/healthz" {
            data = Data()
            status = 200
        } else if parts.count >= 5, parts[4] == "art",
            let id = Int32(parts[3]), parts[2] == "episodes" || parts[2] == "podcasts"
        {
            let path = try await core.artPath(
                id: id, isEpisode: parts[2] == "episodes", small: parts.last == "small")
            if let path {
                data = try await Task.detached { try Data(contentsOf: URL(fileURLWithPath: path)) }.value
                status = 200
            } else {
                data = Data()
                status = 204
            }
        } else {
            let response = try await core.invoke(
                method: request.httpMethod ?? "GET", path: url.path,
                query: URLComponents(url: url, resolvingAgainstBaseURL: false)?.percentEncodedQuery,
                body: request.httpBody)
            data = response.body
            status = Int(response.status)
        }
        guard
            let response = HTTPURLResponse(
                url: url, statusCode: status, httpVersion: nil, headerFields: nil)
        else {
            throw URLError(.badServerResponse)
        }
        return (data, response)
    }
}
