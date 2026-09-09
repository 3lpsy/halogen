import Foundation

/// Typed API requests over the shared wire contract, dispatched through HTTP or local FFI.
@MainActor
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
        /// Fired only when the server explicitly rejects the session credentials.
        var onAuthExpired: (() -> Void)?
        var refreshTask: Task<Void, Error>?
        var nextRefreshAttempt = Date.distantPast

        init(token: String?) {
            self.token = token
        }
    }

    let base: URL
    let tokenBox: TokenBox
    let transport: (URLRequest) async throws -> (Data, URLResponse)
    let now: () -> Date

    var token: String? { tokenBox.token }

    /// Restore saved credentials; requests renew them before the server expiry window closes.
    init(
        baseUrl: String, token: String? = nil,
        now: @escaping () -> Date = Date.init,
        transport: @escaping (URLRequest) async throws -> (Data, URLResponse) = LocalTransport.data
    ) {
        self.base = URL(string: baseUrl)!.appendingPathComponent("api/v1")
        self.tokenBox = TokenBox(token: token)
        self.transport = transport
        self.now = now
    }

    /// Unauthenticated liveness probe — `/healthz` lives at the app root,
    /// outside `/api/v1` (same as `halogen-apiclient`'s health()).
    func health() async throws {
        let root = base.deletingLastPathComponent().deletingLastPathComponent()
        var request = URLRequest(url: root.appendingPathComponent("healthz"))
        request.timeoutInterval = 8
        let (_, response) = try await transport(request)
        guard let response = response as? HTTPURLResponse else { throw URLError(.badServerResponse) }
        let status = response.statusCode
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
        // `data`, query-ish params under `params`) — same as `halogen-apiclient`.
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
        await renewBeforeExpiry(for: request)
        return try await send(request, retryOnAuth: true)
    }

}
