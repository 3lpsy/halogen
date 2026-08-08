import Foundation

/// Admin + account-maintenance slice (poll jobs, server logs/errors,
/// password change). Non-admin callers get clean 403s from the server.
extension HalogenClient {
    func pollJobs() async throws -> [PollJobData] {
        try await get("admin/poll-jobs")
    }

    /// Kick a poll of every feed (or one podcast). Returns the job id.
    func startPollJob(podcastId: Int32? = nil) async throws -> UInt64 {
        var query: [URLQueryItem] = []
        if let podcastId {
            query.append(URLQueryItem(name: "podcast_id", value: String(podcastId)))
        }
        var components = URLComponents(
            url: base.appendingPathComponent("admin/poll-job"),
            resolvingAgainstBaseURL: false
        )!
        if !query.isEmpty { components.queryItems = query }
        var request = URLRequest(url: components.url!)
        request.httpMethod = "POST"
        let envelope: ResponseData<PollJobStartData> = try await send(request)
        guard let data = envelope.data else { throw ClientError.emptyData }
        return data.job_id
    }

    func serverLogs() async throws -> ServerLogsData {
        do {
            return try await get("admin/server-logs")
        } catch let decodeError as DecodingError {
            // Log payloads can arrive with invalid UTF-8 (raw log content),
            // which makes JSONDecoder reject the WHOLE body — refetch raw,
            // re-encode lossily (bad bytes → U+FFFD) and retry.
            var request = URLRequest(url: base.appendingPathComponent("admin/server-logs"))
            request.httpMethod = "GET"
            if let token {
                request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
            }
            let (data, response) = try await URLSession.shared.data(for: request)
            let status = (response as! HTTPURLResponse).statusCode
            guard (200..<300).contains(status) else { throw ClientError.http(status) }
            let lossy = String(decoding: data, as: UTF8.self)
            if let redata = lossy.data(using: .utf8),
                let envelope = try? WireJSON.decoder.decode(
                    ResponseData<ServerLogsData>.self, from: redata),
                let payload = envelope.data
            {
                return payload
            }
            DeviceLog.warn(
                "server-logs decode failed: \(data.count) bytes, "
                    + "head: \(String(decoding: data.prefix(160), as: UTF8.self))")
            throw decodeError
        }
    }

    func serverErrors() async throws -> ServerErrorsData {
        try await get("admin/server-errors")
    }

    /// Change the caller's own password (requires the current one).
    func changePassword(current: String, new: String) async throws {
        try await postEmpty(
            "auth/password",
            body: PasswordChangeData(
                current_password: current,
                new_password: PasswordUpdateData(password: new, password_confirm: new)
            )
        )
    }
}
