import Foundation

extension HalogenClient {
    // Renewal requires a still-valid credential. Waiting for expiry makes refresh impossible.
    func renewBeforeExpiry(for request: URLRequest) async {
        guard request.url?.path.hasSuffix("/auth/login") != true,
            request.url?.path.hasSuffix("/auth/refresh") != true,
            base.scheme != "halogen-local", let token,
            let remaining = remainingLifetime(token), remaining > 0, remaining < 86_400,
            now() >= tokenBox.nextRefreshAttempt
        else { return }
        tokenBox.nextRefreshAttempt = now().addingTimeInterval(min(300, max(5, remaining / 4)))
        do {
            try await refreshToken()
        } catch {
            // A failed renewal must not prevent an otherwise valid request or discard offline access.
            DeviceLog.warn("auth: early renewal unavailable; keeping existing credentials")
        }
    }

    // The unverified expiry is only a scheduling hint. The server verifies every credential.
    func remainingLifetime(_ token: String) -> TimeInterval? {
        let parts = token.split(separator: ".")
        guard parts.count == 3 else { return nil }
        var payload = String(parts[1]).replacingOccurrences(of: "-", with: "+")
            .replacingOccurrences(of: "_", with: "/")
        while payload.count % 4 != 0 { payload += "=" }
        guard let data = Data(base64Encoded: payload),
            let claims = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
            let expiry = claims["exp"] as? Double, expiry.isFinite
        else { return nil }
        return expiry - now().timeIntervalSince1970
    }
}
