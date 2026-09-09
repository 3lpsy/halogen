import Foundation

extension HalogenClient {
    func send<Out: Codable>(
        _ request: URLRequest, retryOnAuth: Bool
    ) async throws -> ResponseData<Out> {
        var request = request
        let sentToken = token
        if let token = sentToken {
            request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        }
        // Every request failure lands in the device log (method + path only,
        // never bodies/tokens) — the app's screens each surface errors their
        // own way, so this is the one place a bug report can see them all.
        let data: Data
        let response: URLResponse
        do {
            (data, response) = try await transport(request)
        } catch {
            DeviceLog.warn(
                "net: \(request.httpMethod ?? "GET") \(request.url?.path ?? "?") — \(error.localizedDescription)"
            )
            throw error
        }
        guard let response = response as? HTTPURLResponse else { throw URLError(.badServerResponse) }
        let status = response.statusCode
        if !(200..<300).contains(status), status != 401 || !retryOnAuth {
            DeviceLog.warn(
                "net: \(request.httpMethod ?? "GET") \(request.url?.path ?? "?") — HTTP \(status)"
            )
        }

        // Refresh only a rejected authenticated request, never a login or refresh itself.
        if status == 401, let sentToken,
            request.url?.path.hasSuffix("/auth/refresh") != true,
            request.url?.path.hasSuffix("/auth/login") != true
        {
            if retryOnAuth {
                // Another request may already have replaced the token while this one was in flight.
                if token == sentToken { try await refreshToken() }
                return try await send(request, retryOnAuth: false)
            }
            if token == sentToken { tokenBox.onAuthExpired?() }
        }

        let envelope: ResponseData<Out>
        do {
            envelope = try WireJSON.decoder.decode(ResponseData<Out>.self, from: data)
        } catch {
            // Non-envelope body (proxy error page, empty 500): the status is
            // the more useful signal than the decode failure.
            guard (200..<300).contains(status) else { throw ClientError.http(status) }
            DeviceLog.warn(
                "net: decode failed for \(request.url?.path ?? "?") — \(error)")
            throw error
        }
        if !(200..<300).contains(status) {
            // The STATUS drives the retry taxonomy: the server envelopes every
            // error, so an errors body must not shadow a status that means
            // retry — classifying an enveloped 401/500 as permanent `.api`
            // dead-letters queued offline mutations (web classify.rs).
            if let errors = envelope.errors, status < 500,
                ![401, 408, 429].contains(status)
            {
                throw ClientError.api(errors)
            }
            throw ClientError.http(status)
        }
        if let errors = envelope.errors { throw ClientError.api(errors) }
        return envelope
    }

    // Share refresh work across concurrent requests. Transient failures keep the saved session.
    func refreshToken() async throws {
        if let task = tokenBox.refreshTask { return try await task.value }
        guard let current = token else { throw ClientError.signedOut }
        let task = Task { try await performRefresh(current: current) }
        tokenBox.refreshTask = task
        defer { tokenBox.refreshTask = nil }
        try await task.value
    }

    private func performRefresh(current: String) async throws {
        var request = URLRequest(url: base.appendingPathComponent("auth/refresh"))
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = try WireJSON.encoder.encode(
            RequestData<TokenData, DefaultDataType>(data: TokenData(token: current), params: nil))
        let (data, response) = try await transport(request)
        guard let response = response as? HTTPURLResponse else { throw URLError(.badServerResponse) }
        let status = response.statusCode
        guard (200..<300).contains(status) else {
            // Only an explicit rejection proves these credentials no longer work.
            if [401, 403].contains(status), token == current { tokenBox.onAuthExpired?() }
            throw ClientError.http(status)
        }
        let envelope = try WireJSON.decoder.decode(ResponseData<TokenData>.self, from: data)
        guard let fresh = envelope.data, !fresh.token.isEmpty else { throw ClientError.emptyData }
        guard token == current else { return }
        tokenBox.token = fresh.token
        if let remaining = remainingLifetime(fresh.token), remaining > 0 {
            tokenBox.nextRefreshAttempt = now().addingTimeInterval(max(1, remaining / 2))
        }
        tokenBox.onRefresh?(fresh.token)
        DeviceLog.info("auth: token refreshed")
    }
}
