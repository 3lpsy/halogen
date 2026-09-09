import XCTest

@testable import Halogen

@MainActor
final class AuthRecoveryTests: XCTestCase {
    private func response(_ request: URLRequest, status: Int, body: String = "{}") -> (Data, URLResponse) {
        (Data(body.utf8), HTTPURLResponse(url: request.url!, statusCode: status, httpVersion: nil, headerFields: nil)!)
    }

    func testRefreshNetworkFailureKeepsSession() async {
        for code in [URLError.notConnectedToInternet, .timedOut, .networkConnectionLost, .cancelled] {
            var expired = false
            let client = HalogenClient(baseUrl: "https://example.invalid", token: "saved") { request in
                if request.url!.path.hasSuffix("/auth/refresh") { throw URLError(code) }
                return self.response(request, status: 401)
            }
            client.tokenBox.onAuthExpired = { expired = true }
            do {
                let _: TokenData = try await client.get("test")
                XCTFail("unreachable refresh should fail the request")
            } catch {
                XCTAssertEqual((error as? URLError)?.code, code)
            }
            XCTAssertFalse(expired)
            XCTAssertEqual(client.token, "saved")
        }
    }

    func testRefreshServerFailureKeepsSession() async {
        for status in [408, 429, 500, 502, 503] {
            var expired = false
            let client = HalogenClient(baseUrl: "https://example.invalid", token: "saved") { request in
                self.response(request, status: request.url!.path.hasSuffix("/auth/refresh") ? status : 401)
            }
            client.tokenBox.onAuthExpired = { expired = true }
            do {
                let _: TokenData = try await client.get("test")
                XCTFail("failed refresh should fail the request")
            } catch HalogenClient.ClientError.http(let actual) {
                XCTAssertEqual(actual, status)
            } catch { XCTFail("unexpected error: \(error)") }
            XCTAssertFalse(expired)
            XCTAssertEqual(client.token, "saved")
        }
    }

    func testMalformedRefreshKeepsSession() async {
        var expired = false
        let client = HalogenClient(baseUrl: "https://example.invalid", token: "saved") { request in
            self.response(request, status: request.url!.path.hasSuffix("/auth/refresh") ? 200 : 401, body: "invalid")
        }
        client.tokenBox.onAuthExpired = { expired = true }
        do {
            let _: TokenData = try await client.get("test")
            XCTFail("invalid refresh response should fail")
        } catch { XCTAssertTrue(error is DecodingError) }
        XCTAssertFalse(expired)
        XCTAssertEqual(client.token, "saved")
    }

    func testRejectedRefreshExpiresSession() async {
        for status in [401, 403] {
            var expirations = 0
            let client = HalogenClient(baseUrl: "https://example.invalid", token: "saved") { request in
                self.response(request, status: request.url!.path.hasSuffix("/auth/refresh") ? status : 401)
            }
            client.tokenBox.onAuthExpired = { expirations += 1 }
            do {
                let _: TokenData = try await client.get("test")
                XCTFail("rejected credentials should fail")
            } catch {}
            XCTAssertEqual(expirations, 1)
        }
    }

    func testConcurrentRequestsShareRefreshAndRetry() async throws {
        var refreshes = 0
        var persisted: [String] = []
        let client = HalogenClient(baseUrl: "https://example.invalid", token: "saved") { request in
            if request.url!.path.hasSuffix("/auth/refresh") {
                refreshes += 1
                try await Task.sleep(nanoseconds: 20_000_000)
                return self.response(request, status: 200, body: "{\"data\":{\"token\":\"fresh\"}}")
            }
            if request.value(forHTTPHeaderField: "Authorization") == "Bearer fresh" {
                return self.response(request, status: 200, body: "{\"data\":{\"token\":\"result\"}}")
            }
            return self.response(request, status: 401)
        }
        client.tokenBox.onRefresh = { persisted.append($0) }
        client.tokenBox.onAuthExpired = { XCTFail("successful refresh must not expire session") }
        async let first: TokenData = client.get("one")
        async let second: TokenData = client.get("two")
        let values = try await [first, second]
        XCTAssertEqual(values.map(\.token), ["result", "result"])
        XCTAssertEqual(refreshes, 1)
        XCTAssertEqual(persisted, ["fresh"])
    }

    func testLateUnauthorizedResponseUsesAlreadyRefreshedToken() async throws {
        var refreshes = 0
        let client = HalogenClient(baseUrl: "https://example.invalid", token: "saved") { request in
            if request.url!.path.hasSuffix("/auth/refresh") {
                refreshes += 1
                return self.response(request, status: 200, body: "{\"data\":{\"token\":\"fresh\"}}")
            }
            if request.value(forHTTPHeaderField: "Authorization") == "Bearer fresh" {
                return self.response(request, status: 200, body: "{\"data\":{\"token\":\"result\"}}")
            }
            if request.url!.path.hasSuffix("/slow") { try await Task.sleep(nanoseconds: 20_000_000) }
            return self.response(request, status: 401)
        }
        client.tokenBox.onAuthExpired = { XCTFail("stale response must not expire session") }
        async let slow: TokenData = client.get("slow")
        async let fast: TokenData = client.get("fast")
        _ = try await [slow, fast]
        XCTAssertEqual(refreshes, 1)
    }
    private func token(expiry: Double) -> String {
        let payload = Data("{\"exp\":\(expiry)}".utf8).base64EncodedString()
            .replacingOccurrences(of: "+", with: "-").replacingOccurrences(of: "/", with: "_")
            .replacingOccurrences(of: "=", with: "")
        return "header.\(payload).signature"
    }

    func testEarlyRenewalExtendsActiveSessionAndThrottlesShortTokens() async throws {
        var time = Date(timeIntervalSince1970: 1_000)
        let old = token(expiry: 1_100)
        let fresh = token(expiry: 1_200)
        var refreshes = 0
        let client = HalogenClient(baseUrl: "https://example.invalid", token: old, now: { time }) { request in
            if request.url!.path.hasSuffix("/auth/refresh") {
                refreshes += 1
                return self.response(request, status: 200, body: "{\"data\":{\"token\":\"\(fresh)\"}}")
            }
            XCTAssertEqual(request.value(forHTTPHeaderField: "Authorization"), "Bearer \(fresh)")
            return self.response(request, status: 200, body: "{\"data\":{\"token\":\"result\"}}")
        }
        let _: TokenData = try await client.get("test")
        time = time.addingTimeInterval(5)
        let _: TokenData = try await client.get("test")
        XCTAssertEqual(refreshes, 1)
        XCTAssertEqual(client.token, fresh)
    }

    func testEarlyRenewalFailureAllowsValidRequestAndThrottlesRetries() async throws {
        var time = Date(timeIntervalSince1970: 1_000)
        let old = token(expiry: 1_100)
        var refreshes = 0
        let client = HalogenClient(baseUrl: "https://example.invalid", token: old, now: { time }) { request in
            if request.url!.path.hasSuffix("/auth/refresh") {
                refreshes += 1
                throw URLError(.networkConnectionLost)
            }
            XCTAssertEqual(request.value(forHTTPHeaderField: "Authorization"), "Bearer \(old)")
            return self.response(request, status: 200, body: "{\"data\":{\"token\":\"result\"}}")
        }
        client.tokenBox.onAuthExpired = { XCTFail("transport failures cannot expire the session") }
        let _: TokenData = try await client.get("test")
        let _: TokenData = try await client.get("test")
        XCTAssertEqual(refreshes, 1)
        time = time.addingTimeInterval(30)
        let _: TokenData = try await client.get("test")
        XCTAssertEqual(refreshes, 2)
        XCTAssertEqual(client.token, old)
    }

}
