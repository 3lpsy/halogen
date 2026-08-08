import Foundation

/// Minimal typed API client over the generated wire types (WireTypes.swift —
/// `just ios-wire-types`). URLSession transport; every API lives under
/// `/api/v1`; auth is `Authorization: Bearer <jwt>` from `/auth/login`.
/// Mirrors the shape of the Rust `halogen-api` client, deliberately thin.
struct HalogenClient {
    enum ClientError: Error, CustomStringConvertible {
        case http(Int)
        case api([String: [ValidationErrorField]])
        case emptyData
        /// Manual offline mode: the request was refused before touching the
        /// network (core.requireClient). Classified transient by the outbox.
        case offline
        /// No client mounted (signed out / session torn down mid-request).
        case signedOut

        var description: String {
            switch self {
            case .http(let code): return "HTTP \(code)"
            case .api(let errors): return "API errors: \(errors)"
            case .emptyData: return "response had no data payload"
            case .offline: return "offline (manual)"
            case .signedOut: return "not signed in"
            }
        }
    }

    /// One page of a list plus whether more pages follow (from the envelope's
    /// paginator; falls back to "a full page implies more" when absent).
    struct PageOf<T> {
        let items: [T]
        let hasMore: Bool
    }

    /// The JWT, shared by REFERENCE across struct copies so an automatic
    /// refresh (or login) propagates everywhere — including copies the core
    /// hands to models. `onRefresh` lets the session store / art loader
    /// follow along.
    final class TokenBox {
        var token: String?
        var onRefresh: ((String) -> Void)?
        /// Fired when a 401 survives the silent refresh — auth is dead (the
        /// web's `ToastDecision::SignOut` / worker `auth_expired` signal).
        /// The core reacts (embedded re-login / back to the connect page);
        /// the request still throws its 401 to the caller.
        var onAuthExpired: (() -> Void)?

        init(token: String?) {
            self.token = token
        }
    }

    let base: URL
    let tokenBox: TokenBox

    var token: String? { tokenBox.token }

    /// `token` restores a previous session's JWT; expiry self-heals via the
    /// 401→refresh→retry path in `send`.
    init(baseUrl: String, token: String? = nil) {
        self.base = URL(string: baseUrl)!.appendingPathComponent("api/v1")
        self.tokenBox = TokenBox(token: token)
    }

    /// Unauthenticated liveness probe — `/healthz` lives at the app root,
    /// outside `/api/v1` (same as `halogen-api`'s health()).
    func health() async throws {
        let root = base.deletingLastPathComponent().deletingLastPathComponent()
        var request = URLRequest(url: root.appendingPathComponent("healthz"))
        request.timeoutInterval = 8
        let (_, response) = try await URLSession.shared.data(for: request)
        let status = (response as! HTTPURLResponse).statusCode
        guard (200..<300).contains(status) else { throw ClientError.http(status) }
    }

    /// Authenticate; returns the raw JWT (the caller derives the account
    /// identity from its `sub` claim — see AccountContext).
    @discardableResult
    func login(username: String, password: String) async throws -> String {
        let token: TokenData = try await post(
            "auth/login",
            body: LoginData(username: username, password: password)
        )
        tokenBox.token = token.token
        return token.token
    }

    /// One page of the library (server default page size is 10 — always be
    /// explicit or big libraries silently truncate).
    func podcasts(page: Int = 0, pageSize: Int = 20) async throws -> PageOf<PodcastData> {
        let envelope: ResponseData<[PodcastData]> = try await getEnvelope(
            "podcasts",
            query: [
                URLQueryItem(name: "pagination[page]", value: String(page)),
                URLQueryItem(name: "pagination[size]", value: String(pageSize)),
            ])
        guard let items = envelope.data else { throw ClientError.emptyData }
        let hasMore: Bool
        if let paginator = envelope.paginator {
            hasMore = paginator.page + 1 < paginator.pages
        } else {
            hasMore = items.count == pageSize
        }
        return PageOf(items: items, hasMore: hasMore)
    }

    /// One podcast by id — the deep-link fetch-through (web `get_podcast`):
    /// navigating to a podcast that isn't in the cached library pool loads
    /// it on the fly instead of dead-ending.
    func podcast(id: Int32) async throws -> PodcastData {
        try await get("podcasts/\(id)")
    }

    /// Episodes of one podcast, newest first. Query params mirror the wire
    /// `DefaultListParams` shape serde_qs expects (`filter[podcast_id]`, …).
    func episodes(
        podcastId: Int32, extra: [URLQueryItem] = [], page: Int = 0, pageSize: Int = 20
    ) async throws -> PageOf<EpisodeData> {
        var items = [
            URLQueryItem(name: "pagination[page]", value: String(page)),
            URLQueryItem(name: "pagination[size]", value: String(pageSize)),
            URLQueryItem(name: "filter[podcast_id]", value: String(podcastId)),
            // Resume cursors ride every episode page (web parity).
            URLQueryItem(name: "includes[0]", value: "Playback"),
        ]
        // Caller's order wins; default newest-first.
        if !extra.contains(where: { $0.name == "order[order_by]" }) {
            items.append(URLQueryItem(name: "order[direction]", value: "Desc"))
            items.append(URLQueryItem(name: "order[order_by]", value: "published_at"))
        }
        let envelope: ResponseData<[EpisodeData]> = try await getEnvelope(
            "episodes", query: items + extra)
        guard let payload = envelope.data else { throw ClientError.emptyData }
        let hasMore: Bool
        if let paginator = envelope.paginator {
            hasMore = paginator.page + 1 < paginator.pages
        } else {
            hasMore = payload.count == pageSize
        }
        return PageOf(items: payload, hasMore: hasMore)
    }

    /// One page of the newest episodes across the whole library, parent
    /// podcast embedded (`includes[0]=Podcast`) so rows can name their show.
    /// `filter` appends a wire `FilterParams` fragment (facet chips).
    func latestEpisodes(
        filter: [URLQueryItem] = [],
        page: Int = 0,
        pageSize: Int = 20
    ) async throws -> PageOf<EpisodeData> {
        var items = [
            URLQueryItem(name: "pagination[page]", value: String(page)),
            URLQueryItem(name: "pagination[size]", value: String(pageSize)),
            URLQueryItem(name: "includes[0]", value: "Podcast"),
            // Resume cursors ride every episode page (web:
            // EpisodeInclude::Playback) so playing from a list resumes
            // and rows can show progress.
            URLQueryItem(name: "includes[1]", value: "Playback"),
        ]
        // Caller's order wins; default newest-first. Duplicated order keys
        // parse as a sequence server-side and 400 the whole request.
        if !filter.contains(where: { $0.name == "order[order_by]" }) {
            items.append(URLQueryItem(name: "order[direction]", value: "Desc"))
            items.append(URLQueryItem(name: "order[order_by]", value: "published_at"))
        }
        let envelope: ResponseData<[EpisodeData]> = try await getEnvelope(
            "episodes", query: items + filter
        )
        guard let items = envelope.data else { throw ClientError.emptyData }
        let hasMore: Bool
        if let paginator = envelope.paginator {
            hasMore = paginator.page + 1 < paginator.pages
        } else {
            hasMore = items.count == pageSize
        }
        return PageOf(items: items, hasMore: hasMore)
    }

    // MARK: - transport

    func get<Out: Codable>(_ path: String, query: [URLQueryItem] = []) async throws -> Out {
        let envelope: ResponseData<Out> = try await getEnvelope(path, query: query)
        guard let payload = envelope.data else { throw ClientError.emptyData }
        return payload
    }

    func getEnvelope<Out: Codable>(
        _ path: String,
        query: [URLQueryItem] = []
    ) async throws -> ResponseData<Out> {
        var components = URLComponents(
            url: base.appendingPathComponent(path),
            resolvingAgainstBaseURL: false
        )!
        if !query.isEmpty { components.queryItems = query }
        var request = URLRequest(url: components.url!)
        request.httpMethod = "GET"
        return try await send(request)
    }

    func post<In: Codable, Out: Codable>(_ path: String, body: In) async throws -> Out {
        var request = URLRequest(url: base.appendingPathComponent(path))
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        // Every mutating endpoint takes the RequestData envelope (body under
        // `data`, query-ish params under `params`) — same as `halogen-api`.
        request.httpBody = try WireJSON.encoder.encode(
            RequestData<In, DefaultDataType>(data: body, params: nil)
        )
        let envelope: ResponseData<Out> = try await send(request)
        guard let payload = envelope.data else { throw ClientError.emptyData }
        return payload
    }

    /// POST whose success payload is irrelevant (`data` may be null).
    func postEmpty<In: Codable>(_ path: String, body: In) async throws {
        var request = URLRequest(url: base.appendingPathComponent(path))
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = try WireJSON.encoder.encode(
            RequestData<In, DefaultDataType>(data: body, params: nil)
        )
        let _: ResponseData<DefaultDataType> = try await send(request)
    }

    func delete(_ path: String) async throws {
        var request = URLRequest(url: base.appendingPathComponent(path))
        request.httpMethod = "DELETE"
        let _: ResponseData<DefaultDataType> = try await send(request)
    }

    func send<Out: Codable>(_ request: URLRequest) async throws -> ResponseData<Out> {
        try await send(request, retryOnAuth: true)
    }

    private func send<Out: Codable>(
        _ request: URLRequest, retryOnAuth: Bool
    ) async throws -> ResponseData<Out> {
        var request = request
        if let token {
            request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        }
        // Every request failure lands in the device log (method + path only,
        // never bodies/tokens) — the app's screens each surface errors their
        // own way, so this is the one place a bug report can see them all.
        let data: Data
        let response: URLResponse
        do {
            (data, response) = try await URLSession.shared.data(for: request)
        } catch {
            DeviceLog.warn(
                "net: \(request.httpMethod ?? "GET") \(request.url?.path ?? "?") — \(error.localizedDescription)"
            )
            throw error
        }
        let status = (response as! HTTPURLResponse).statusCode
        if !(200..<300).contains(status), status != 401 || !retryOnAuth {
            DeviceLog.warn(
                "net: \(request.httpMethod ?? "GET") \(request.url?.path ?? "?") — HTTP \(status)"
            )
        }

        // Expired/stale token: refresh once and retry (live remote servers —
        // the embedded server's tokens rarely age out within a session).
        if status == 401, retryOnAuth, token != nil,
            request.url?.path.hasSuffix("/auth/refresh") != true,
            request.url?.path.hasSuffix("/auth/login") != true
        {
            if await refreshToken() {
                return try await send(request, retryOnAuth: false)
            }
            // Refresh couldn't heal it: the session is genuinely expired or
            // revoked. Tell the core before the 401 propagates.
            tokenBox.onAuthExpired?()
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

    /// POST /auth/refresh with the current token; true on success (box +
    /// listeners updated).
    private func refreshToken() async -> Bool {
        guard let current = token else { return false }
        var request = URLRequest(url: base.appendingPathComponent("auth/refresh"))
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        guard
            let body = try? WireJSON.encoder.encode(
                RequestData<TokenData, DefaultDataType>(
                    data: TokenData(token: current), params: nil))
        else { return false }
        request.httpBody = body
        // The failure MODE matters downstream: a rejected refresh means the
        // session is dead; a transport failure means it might not be — log
        // which one preceded a forced sign-out.
        guard let (data, response) = try? await URLSession.shared.data(for: request) else {
            DeviceLog.warn("auth: token refresh unreachable (transport)")
            return false
        }
        guard
            (response as? HTTPURLResponse).map({ (200..<300).contains($0.statusCode) }) == true,
            let envelope = try? WireJSON.decoder.decode(
                ResponseData<TokenData>.self, from: data),
            let fresh = envelope.data
        else {
            DeviceLog.warn(
                "auth: token refresh rejected (status \((response as? HTTPURLResponse)?.statusCode ?? -1))"
            )
            return false
        }
        tokenBox.token = fresh.token
        tokenBox.onRefresh?(fresh.token)
        DeviceLog.info("auth: token refreshed")
        return true
    }
}
