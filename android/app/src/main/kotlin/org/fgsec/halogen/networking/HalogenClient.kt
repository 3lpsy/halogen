package org.fgsec.halogen.networking

import java.util.concurrent.TimeUnit
import kotlinx.serialization.KSerializer
import kotlinx.serialization.SerializationException
import kotlinx.serialization.builtins.ListSerializer
import okhttp3.HttpUrl
import okhttp3.HttpUrl.Companion.toHttpUrl
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import org.fgsec.halogen.core.DeviceLog
import org.fgsec.halogen.wire.DefaultDataType
import org.fgsec.halogen.wire.EpisodeData
import org.fgsec.halogen.wire.LoginData
import org.fgsec.halogen.wire.PodcastData
import org.fgsec.halogen.wire.RequestData
import org.fgsec.halogen.wire.ResponseData
import org.fgsec.halogen.wire.TokenData
import org.fgsec.halogen.wire.ValidationErrorField

/// Minimal typed API client over the generated wire types (WireTypes.kt —
/// `just android-wire-types`). OkHttp transport; every API lives under
/// `/api/v1`; auth is `Authorization: Bearer <jwt>` from `/auth/login`.
/// Mirrors the shape of the Rust `halogen-api` client, deliberately thin.
class HalogenClient(baseUrl: String, token: String? = null) {
    sealed class ClientError(message: String) : Exception(message) {
        class Http(val code: Int) : ClientError("HTTP $code")
        class Api(val errors: Map<String, List<ValidationErrorField>>) :
            ClientError("API errors: $errors")
        object EmptyData : ClientError("response had no data payload")
        /// Manual offline mode: the request was refused before touching the
        /// network (core.requireClient). Classified transient by the outbox.
        object Offline : ClientError("offline (manual)")
        /// No client mounted (signed out / session torn down mid-request).
        object SignedOut : ClientError("not signed in")
    }

    /// One page of a list plus whether more pages follow (from the envelope's
    /// paginator; falls back to "a full page implies more" when absent).
    data class PageOf<T>(val items: List<T>, val hasMore: Boolean)

    private val root: HttpUrl = baseUrl.trimEnd('/').toHttpUrl()
    val base: HttpUrl = root.newBuilder().addPathSegments("api/v1").build()
    val tokenBox: TokenBox = TokenBox(token)

    val token: String? get() = tokenBox.token

    /// Unauthenticated liveness probe — `/healthz` lives at the app root,
    /// outside `/api/v1` (same as `halogen-api`'s health()).
    suspend fun health() {
        val request = Request.Builder()
            .url(root.newBuilder().addPathSegment("healthz").build())
            .get()
            .build()
        val client = Http.client.newBuilder().callTimeout(8, TimeUnit.SECONDS).build()
        client.newCall(request).await().use { response ->
            if (response.code !in 200..299) throw ClientError.Http(response.code)
        }
    }

    /// Authenticate; returns the raw JWT (the caller derives the account
    /// identity from its `sub` claim — see AccountContext).
    suspend fun login(username: String, password: String): String {
        val token: TokenData = post(
            "auth/login",
            LoginData(username = username, password = password),
            LoginData.serializer(),
            TokenData.serializer(),
        )
        tokenBox.token = token.token
        return token.token
    }

    /// One page of the library (server default page size is 10 — always be
    /// explicit or big libraries silently truncate).
    suspend fun podcasts(page: Int = 0, pageSize: Int = 20): PageOf<PodcastData> {
        val envelope = getEnvelope(
            "podcasts",
            listOf(
                "pagination[page]" to page.toString(),
                "pagination[size]" to pageSize.toString(),
            ),
            ListSerializer(PodcastData.serializer()),
        )
        return pageOf(envelope, pageSize)
    }

    /// One podcast by id — the deep-link fetch-through (web `get_podcast`):
    /// navigating to a podcast that isn't in the cached library pool loads
    /// it on the fly instead of dead-ending.
    suspend fun podcast(id: Int): PodcastData =
        get("podcasts/$id", emptyList(), PodcastData.serializer())

    /// Episodes of one podcast, newest first. Query params mirror the wire
    /// `DefaultListParams` shape serde_qs expects (`filter[podcast_id]`, …).
    suspend fun episodes(
        podcastId: Int,
        extra: List<Pair<String, String>> = emptyList(),
        page: Int = 0,
        pageSize: Int = 20,
    ): PageOf<EpisodeData> {
        val items = mutableListOf(
            "pagination[page]" to page.toString(),
            "pagination[size]" to pageSize.toString(),
            "filter[podcast_id]" to podcastId.toString(),
            // Resume cursors ride every episode page (web parity).
            "includes[0]" to "Playback",
        )
        // Caller's order wins; default newest-first.
        if (extra.none { it.first == "order[order_by]" }) {
            items.add("order[direction]" to "Desc")
            items.add("order[order_by]" to "published_at")
        }
        val envelope = getEnvelope(
            "episodes", items + extra, ListSerializer(EpisodeData.serializer()))
        return pageOf(envelope, pageSize)
    }

    /// One page of the newest episodes across the whole library, parent
    /// podcast embedded (`includes[0]=Podcast`) so rows can name their show.
    /// `filter` appends a wire `FilterParams` fragment (facet chips).
    suspend fun latestEpisodes(
        filter: List<Pair<String, String>> = emptyList(),
        page: Int = 0,
        pageSize: Int = 20,
    ): PageOf<EpisodeData> {
        val items = mutableListOf(
            "pagination[page]" to page.toString(),
            "pagination[size]" to pageSize.toString(),
            "includes[0]" to "Podcast",
            // Resume cursors ride every episode page (web:
            // EpisodeInclude::Playback) so playing from a list resumes
            // and rows can show progress.
            "includes[1]" to "Playback",
        )
        // Caller's order wins; default newest-first. Duplicated order keys
        // parse as a sequence server-side and 400 the whole request.
        if (filter.none { it.first == "order[order_by]" }) {
            items.add("order[direction]" to "Desc")
            items.add("order[order_by]" to "published_at")
        }
        val envelope = getEnvelope(
            "episodes", items + filter, ListSerializer(EpisodeData.serializer()))
        return pageOf(envelope, pageSize)
    }

    /// The shared paginator fallback for list endpoints.
    fun <T> pageOf(envelope: ResponseData<List<T>>, pageSize: Int): PageOf<T> {
        val items = envelope.data ?: throw ClientError.EmptyData
        val paginator = envelope.paginator
        val hasMore =
            if (paginator != null) paginator.page + 1 < paginator.pages
            else items.size == pageSize
        return PageOf(items, hasMore)
    }

    // MARK: - transport

    fun url(path: String, query: List<Pair<String, String>> = emptyList()): HttpUrl {
        val builder = base.newBuilder().addPathSegments(path)
        for ((name, value) in query) builder.addQueryParameter(name, value)
        return builder.build()
    }

    suspend fun <Out> get(
        path: String,
        query: List<Pair<String, String>> = emptyList(),
        serializer: KSerializer<Out>,
    ): Out {
        val envelope = getEnvelope(path, query, serializer)
        return envelope.data ?: throw ClientError.EmptyData
    }

    suspend fun <Out> getEnvelope(
        path: String,
        query: List<Pair<String, String>> = emptyList(),
        serializer: KSerializer<Out>,
    ): ResponseData<Out> {
        val request = Request.Builder().url(url(path, query)).get().build()
        return send(request, serializer)
    }

    /// The RequestData envelope every mutating endpoint takes (body under
    /// `data`, query-ish params under `params`) — same as `halogen-api`.
    fun <In> envelopeBody(body: In, serializer: KSerializer<In>): okhttp3.RequestBody =
        WireJson.json
            .encodeToString(
                RequestData.serializer(serializer, DefaultDataType.serializer()),
                RequestData(data = body, params = null),
            )
            .toRequestBody("application/json".toMediaType())

    suspend fun <In, Out> post(
        path: String,
        body: In,
        bodySerializer: KSerializer<In>,
        serializer: KSerializer<Out>,
    ): Out {
        val request = Request.Builder()
            .url(url(path))
            .post(envelopeBody(body, bodySerializer))
            .build()
        val envelope = send(request, serializer)
        return envelope.data ?: throw ClientError.EmptyData
    }

    /// POST whose success payload is irrelevant (`data` may be null).
    suspend fun <In> postEmpty(path: String, body: In, bodySerializer: KSerializer<In>) {
        val request = Request.Builder()
            .url(url(path))
            .post(envelopeBody(body, bodySerializer))
            .build()
        send(request, DefaultDataType.serializer())
    }

    suspend fun <In, Out> put(
        path: String,
        body: In,
        bodySerializer: KSerializer<In>,
        serializer: KSerializer<Out>,
    ): Out {
        val request = Request.Builder()
            .url(url(path))
            .put(envelopeBody(body, bodySerializer))
            .build()
        val envelope = send(request, serializer)
        return envelope.data ?: throw ClientError.EmptyData
    }

    suspend fun delete(path: String) {
        val request = Request.Builder().url(url(path)).delete().build()
        send(request, DefaultDataType.serializer())
    }

    suspend fun <Out> send(request: Request, serializer: KSerializer<Out>): ResponseData<Out> =
        send(request, serializer, retryOnAuth = true)

    private suspend fun <Out> send(
        request: Request,
        serializer: KSerializer<Out>,
        retryOnAuth: Boolean,
    ): ResponseData<Out> {
        var authed = request
        token?.let { authed = request.newBuilder().header("Authorization", "Bearer $it").build() }
        // Every request failure lands in the device log (method + path only,
        // never bodies/tokens) — the app's screens each surface errors their
        // own way, so this is the one place a bug report can see them all.
        val http = if (request.method == "GET") Http.client else Http.mutationClient
        val response = try {
            http.newCall(authed).await()
        } catch (e: Exception) {
            DeviceLog.warn("net: ${request.method} ${request.url.encodedPath} — ${e.message}")
            throw e
        }
        val status = response.code
        val body = try {
            response.readBody()
        } catch (e: Exception) {
            DeviceLog.warn("net: body read failed for ${request.url.encodedPath} — ${e.message}")
            throw e
        }
        if (status !in 200..299 && (status != 401 || !retryOnAuth)) {
            DeviceLog.warn("net: ${request.method} ${request.url.encodedPath} — HTTP $status")
        }

        // Expired/stale token: refresh once and retry (live remote servers —
        // the embedded server's tokens rarely age out within a session).
        if (status == 401 && retryOnAuth && token != null &&
            !request.url.encodedPath.endsWith("/auth/refresh") &&
            !request.url.encodedPath.endsWith("/auth/login")
        ) {
            if (refreshToken()) {
                return send(request, serializer, retryOnAuth = false)
            }
            // Refresh couldn't heal it: the session is genuinely expired or
            // revoked. Tell the core before the 401 propagates.
            tokenBox.onAuthExpired?.invoke()
        }

        val envelope: ResponseData<Out>
        try {
            envelope = WireJson.json.decodeFromString(ResponseData.serializer(serializer), body)
        } catch (e: Exception) {
            // Non-envelope body (proxy error page, empty 500): the status beats the
            // decode failure. Catch every exception type — custom serializers can throw
            // outside SerializationException, and unlogged failures are undiagnosable.
            if (status !in 200..299) throw ClientError.Http(status)
            DeviceLog.warn("net: decode failed for ${request.url.encodedPath} — $e")
            throw e
        }
        if (status !in 200..299) {
            // The STATUS drives the retry taxonomy — the server envelopes every error,
            // and an errors body must not shadow a retryable status: an enveloped
            // 401/500 classified `.Api` (permanent) would dead-letter queued mutations
            // (web classify.rs: 401/408/429 stay transient, 5xx is budgeted).
            val errors = envelope.errors
            if (errors != null && status < 500 && status !in listOf(401, 408, 429)) {
                throw ClientError.Api(errors)
            }
            throw ClientError.Http(status)
        }
        envelope.errors?.let { throw ClientError.Api(it) }
        return envelope
    }

    /// POST /auth/refresh with the current token; true on success (box +
    /// listeners updated).
    private suspend fun refreshToken(): Boolean {
        val current = token ?: return false
        val request = Request.Builder()
            .url(url("auth/refresh"))
            .post(envelopeBody(TokenData(token = current), TokenData.serializer()))
            .build()
        // The failure MODE matters downstream: a rejected refresh means the
        // session is dead; a transport failure means it might not be — log
        // which one preceded a forced sign-out.
        val response = try {
            Http.mutationClient.newCall(request).await()
        } catch (_: Exception) {
            DeviceLog.warn("auth: token refresh unreachable (transport)")
            return false
        }
        val status = response.code
        val body = try {
            response.readBody()
        } catch (_: Exception) {
            DeviceLog.warn("auth: token refresh body read failed")
            return false
        }
        val fresh = if (status in 200..299) {
            try {
                WireJson.json
                    .decodeFromString(ResponseData.serializer(TokenData.serializer()), body)
                    .data
            } catch (_: SerializationException) {
                null
            }
        } else null
        if (fresh == null) {
            DeviceLog.warn("auth: token refresh rejected (status $status)")
            return false
        }
        tokenBox.token = fresh.token
        tokenBox.onRefresh?.invoke(fresh.token)
        DeviceLog.info("auth: token refreshed")
        return true
    }
}
