//! The typed [`ApiClient`] and its request methods.

use futures_util::StreamExt;
use reqwest::Client;
use serde::de::DeserializeOwned;
use url::Url;

use halogen_wire::{
    ConfigData, ConfigOverridesData, DbImportSummaryData, DefaultPlaylistData,
    DiscoverProvidersData, DiscoverSearchData, DiscoverSearchParams, EpisodeData, EpisodeInclude,
    EpisodePlaylistBulkData, EpisodePlaylistData, EpisodePlaylistMoveData,
    EpisodePlaylistStoreData, EpisodeUpdateData, LoginData, OpmlExportData, OpmlImportData,
    OpmlImportResultData, Page, PasswordChangeData, PlaybackData, PlaybackListParams,
    PlaybackStoreData, PlaylistData, PlaylistInclude, PlaylistStoreData, PlaylistUpdateData,
    PodcastAutoPlaylistData, PodcastAutoPlaylistSetData, PodcastConfigData, PodcastConfigStoreData,
    PodcastConfigUpdateData, PodcastData, PodcastInclude, PodcastStoreData, PodcastUpdateData,
    PollJobData, PollJobStartData, PollingOperationData, PollingStatusData, ResponsableData,
    ServerErrorsData, ServerLogsData, StatusData, TokenData, UserData, UserStoreData,
    UserUpdateData, WsTicketData,
};

use crate::audio::{AudioChunk, AudioStream};
use crate::error::{ApiError, parse_error_body};
use crate::parse::{parse_content_range_start, parse_content_range_total};

/// Typed API client for the Halogen backend.
///
/// `token` is interior-mutable so the sync worker and auth flow can update it
/// after login/refresh without rebuilding the client. Every authed call
/// attaches `Authorization: Bearer <token>`.
pub struct ApiClient {
    http: Client,
    /// Base URL with any trailing `/` stripped, so `format!("{base}{path}")`
    /// (path starts with `/`) never produces a `//` that fails route matching.
    base: String,
    token: std::sync::RwLock<Option<String>>,
}

/// The underlying HTTP client. Native gets a CONNECT timeout so an unreachable
/// or black-holed link (server down, wrong URL, a captive portal that never
/// completes the TCP handshake — the common "hangs forever" cases) surfaces as
/// a `Transport` error in bounded time instead of hanging sync, login, and
/// downloads indefinitely.
///
/// Deliberately NO read/total timeout: this ONE client serves both streaming
/// audio downloads (a legitimate 100+ MB transfer on a slow link outlives any
/// fixed cap) AND server-side-BLOCKING endpoints like `/admin/poll`, which hold
/// the connection open with no response bytes while the server fetches feeds —
/// an idle-read timeout kills those exactly when they're working. The
/// connect timeout covers the dominant failure mode without that collateral
/// damage. wasm rides the browser's fetch stack (no builder timeouts exposed).
fn build_http() -> Client {
    #[cfg(not(target_arch = "wasm32"))]
    {
        Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .build()
            .unwrap_or_else(|_| Client::new())
    }
    #[cfg(target_arch = "wasm32")]
    {
        Client::new()
    }
}

impl ApiClient {
    /// Create a new `ApiClient` pointing at `base_url`.
    ///
    /// All HTTP APIs live under `/api/v1` (so they never shadow the SPA's
    /// client-side routes), so the prefix is baked into the base here — every
    /// path method below stays prefix-free.
    pub fn new(base_url: Url) -> Self {
        Self {
            http: build_http(),
            base: format!("{}/api/v1", base_url.as_str().trim_end_matches('/')),
            token: std::sync::RwLock::new(None),
        }
    }

    /// An independent client with the same base URL and a snapshot of the
    /// current token. For detached tasks (e.g. the device-download fetch) that
    /// outlive the borrow of the owner's client. Token updates after this call
    /// don't propagate — fine for short-lived tasks.
    pub fn clone_handle(&self) -> Self {
        Self {
            http: build_http(),
            base: self.base.clone(),
            token: std::sync::RwLock::new(self.token()),
        }
    }

    /// Set the auth token (called after login/refresh).
    pub fn set_token(&self, token: Option<String>) {
        *self.token.write().unwrap() = token;
    }

    /// Get the current auth token.
    pub fn token(&self) -> Option<String> {
        self.token.read().unwrap().clone()
    }

    // ── Auth / bootstrap (no auth required) ──────────────────────────────

    /// GET /healthz — unauthenticated liveness probe (used by the login
    /// reachability check). Lives at the app root, outside the `/api/v1` prefix
    /// baked into `self.base`, so strip the prefix to reach it.
    pub async fn health(&self) -> Result<StatusData, ApiError> {
        let origin = self
            .base
            .strip_suffix("/api/v1")
            .unwrap_or(self.base.as_str());
        let url = format!("{origin}/healthz");
        let response = self.http.get(url).send().await?;
        self.handle_single_response(response).await
    }

    /// POST /ws-ticket — mint a short-lived ticket for the connectivity
    /// WebSocket. Authed (bearer): a browser WebSocket handshake can't carry an
    /// `Authorization` header, so the client mints this first and passes it as the
    /// `?ticket=` query param on the `/ws` upgrade. Re-minted on every (re)connect.
    pub async fn mint_ws_ticket(&self) -> Result<WsTicketData, ApiError> {
        let url = format!("{}{}", self.base, "/ws-ticket");
        let response = self.authed_post(url, String::new()).await?;
        self.handle_single_response(response).await
    }

    /// The `ws://`/`wss://` URL for the connectivity socket, derived from the base
    /// (scheme `http→ws` / `https→wss`, path `/api/v1/ws`). The ticket is appended
    /// by the caller — kept off this value so it never lands in a log.
    pub fn ws_url(&self) -> String {
        let base = self.base.as_str();
        if let Some(rest) = base.strip_prefix("https://") {
            format!("wss://{rest}/ws")
        } else if let Some(rest) = base.strip_prefix("http://") {
            format!("ws://{rest}/ws")
        } else {
            // No recognised scheme (shouldn't happen — `new` builds from a `Url`);
            // fall back to a ws:// best-effort rather than panicking.
            format!("ws://{base}/ws")
        }
    }

    /// POST /auth/login — authenticate and return a token.
    pub async fn login(&self, creds: LoginData) -> Result<TokenData, ApiError> {
        let body = halogen_wire::RequestData::<_, ()>::from_data(creds);
        let body_str = serde_json::to_string(&body).map_err(|e| ApiError::Decode(e.to_string()))?;
        let url = format!("{}{}", self.base, "/auth/login");
        let req = self
            .http
            .post(url)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body_str);
        // wasm: opt the fetch into storing the `auth_media` cookie cross-origin
        // (default credentials mode is `same-origin`).
        #[cfg(target_arch = "wasm32")]
        let req = req.fetch_credentials_include();
        let response = req.send().await?;
        self.handle_single_response(response).await
    }

    /// POST /auth/refresh — refresh an existing token.
    pub async fn refresh(&self, token: TokenData) -> Result<TokenData, ApiError> {
        let body = halogen_wire::RequestData::<_, ()>::from_data(token);
        let body_str = serde_json::to_string(&body).map_err(|e| ApiError::Decode(e.to_string()))?;
        let url = format!("{}{}", self.base, "/auth/refresh");
        let req = self
            .http
            .post(url)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body_str);
        // wasm: refresh re-sets the `auth_media` cookie — keep credentials on.
        #[cfg(target_arch = "wasm32")]
        let req = req.fetch_credentials_include();
        let response = req.send().await?;
        self.handle_single_response(response).await
    }

    /// POST /auth/logout — stateless logout.
    pub async fn logout(&self) -> Result<(), ApiError> {
        let url = format!("{}{}", self.base, "/auth/logout");
        let req = self.http.post(url);
        // wasm: needed so the server's cookie-clearing `Set-Cookie` is honoured.
        #[cfg(target_arch = "wasm32")]
        let req = req.fetch_credentials_include();
        let response = req.send().await?;
        self.handle_empty_response(response).await
    }

    // ── Podcasts ─────────────────────────────────────────────────────────

    /// GET /podcasts — list podcasts with pagination.
    pub async fn list_podcasts(
        &self,
        params: halogen_wire::DefaultListParams<PodcastInclude>,
    ) -> Result<Page<Vec<PodcastData>>, ApiError> {
        let qs = serde_qs::to_string(&params).map_err(|e| ApiError::Decode(e.to_string()))?;
        let url = build_url(&self.base, "/podcasts", &qs);
        let response = self.authed_get(url).await?;
        self.handle_response(response).await
    }

    /// GET /podcasts/{id} — always requests the `podcast_config` include so the
    /// returned podcast carries its download/poll override (the config form prefills
    /// from it, and the detail page can render the config offline).
    pub async fn get_podcast(&self, id: i32) -> Result<PodcastData, ApiError> {
        let params = halogen_wire::DefaultGetParams::<PodcastInclude> {
            id: None,
            includes: Some(vec![PodcastInclude::PodcastConfig]),
        };
        let qs = serde_qs::to_string(&params).map_err(|e| ApiError::Decode(e.to_string()))?;
        let path = format!("/podcasts/{}", id);
        let url = build_url(&self.base, &path, &qs);
        let response = self.authed_get(url).await?;
        let resp: Page<PodcastData> = self.handle_response(response).await?;
        Ok(resp.data)
    }

    /// POST /podcasts
    pub async fn create_podcast(&self, data: PodcastStoreData) -> Result<PodcastData, ApiError> {
        let body = halogen_wire::RequestData::<_, ()>::from_data(data);
        let body_str = serde_json::to_string(&body).map_err(|e| ApiError::Decode(e.to_string()))?;
        let url = format!("{}{}", self.base, "/podcasts");
        let response = self.authed_post(url, body_str).await?;
        let resp: Page<PodcastData> = self.handle_response(response).await?;
        Ok(resp.data)
    }

    /// PUT /podcasts/{id}
    pub async fn update_podcast(
        &self,
        id: i32,
        data: PodcastUpdateData,
    ) -> Result<PodcastData, ApiError> {
        let body = halogen_wire::RequestData::<_, ()>::from_data(data);
        let body_str = serde_json::to_string(&body).map_err(|e| ApiError::Decode(e.to_string()))?;
        let url = format!("{}/podcasts/{}", self.base, id);
        let response = self.authed_put(url, body_str).await?;
        let resp: Page<PodcastData> = self.handle_response(response).await?;
        Ok(resp.data)
    }

    /// DELETE /podcasts/{id}
    pub async fn delete_podcast(&self, id: i32) -> Result<(), ApiError> {
        let url = format!("{}/podcasts/{}", self.base, id);
        let response = self.authed_delete(url).await?;
        self.handle_empty_response(response).await?;
        Ok(())
    }

    // ── Episodes ─────────────────────────────────────────────────────────

    /// GET /episodes — list episodes with pagination.
    pub async fn list_episodes(
        &self,
        params: halogen_wire::DefaultListParams<EpisodeInclude>,
    ) -> Result<Page<Vec<EpisodeData>>, ApiError> {
        let qs = serde_qs::to_string(&params).map_err(|e| ApiError::Decode(e.to_string()))?;
        let url = build_url(&self.base, "/episodes", &qs);
        let response = self.authed_get(url).await?;
        self.handle_response(response).await
    }

    /// GET /episodes/{id}. `includes` selects eager-loaded relations (e.g.
    /// `Playback` to embed the caller's resume cursor); empty = no includes.
    pub async fn get_episode(
        &self,
        id: i32,
        includes: &[EpisodeInclude],
    ) -> Result<EpisodeData, ApiError> {
        let path = format!("/episodes/{}", id);
        let qs = if includes.is_empty() {
            String::new()
        } else {
            let params = halogen_wire::EpisodeShowParams {
                id: None,
                podcast_id: None,
                includes: Some(includes.to_vec()),
            };
            serde_qs::to_string(&params).map_err(|e| ApiError::Decode(e.to_string()))?
        };
        let url = build_url(&self.base, &path, &qs);
        let response = self.authed_get(url).await?;
        let resp: Page<EpisodeData> = self.handle_response(response).await?;
        Ok(resp.data)
    }

    /// GET /episodes/{id}/audio — the server's stored copy of an episode as raw
    /// bytes, plus its `Content-Type`. Used by the device-download flow to pull
    /// the file into local byte storage.
    ///
    /// Auth: the normal bearer token, like every other API call — the audio
    /// endpoint accepts bearer (programmatic fetches) OR the `auth_media`
    /// cookie (`<audio>` streaming, which can't set headers).
    ///
    /// The whole file is buffered in memory (`.bytes()`); acceptable for
    /// podcast-sized files. Follow-up if it ever matters: a wasm-only path via
    /// `web_sys::fetch` + `Response.blob()` keeps the bytes out of the wasm heap.
    pub async fn download_audio(&self, id: i32) -> Result<(Vec<u8>, Option<String>), ApiError> {
        let url = format!("{}/episodes/{}/audio", self.base, id);
        let response = self.authed_get(url).await?;
        let status = response.status();
        if !status.is_success() {
            return Err(ApiError::Server {
                status: status.as_u16(),
                message: format!("audio fetch failed ({status})"),
            });
        }
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        let bytes = response.bytes().await?;
        Ok((bytes.to_vec(), content_type))
    }

    /// GET /episodes/{id}/audio with a `Range: bytes={start}-{end}` header — one
    /// chunk of the server's stored copy. Used by the resumable device-download
    /// flow so a flaky connection only re-fetches the chunk it dropped, not the
    /// whole (often 100+ MB) file. `ServeFile` answers with `206 Partial Content`
    /// and a `Content-Range` whose `/total` we parse so the caller knows when it's
    /// done; a server that ignores `Range` answers `200` with the whole body (and
    /// `Content-Length`), which the caller treats as a single-chunk download. The
    /// returned [`AudioChunk`] also carries `served_from` (the offset the body
    /// starts at, like [`download_audio_stream`](Self::download_audio_stream)) so
    /// the caller can detect a server that ignored the range and restart instead
    /// of writing the bytes at the wrong offset.
    pub async fn download_audio_range(
        &self,
        id: i32,
        start: u64,
        end: u64,
    ) -> Result<AudioChunk, ApiError> {
        let url = format!("{}/episodes/{}/audio", self.base, id);
        let mut builder = self.http.get(&url);
        if let Some(token) = self.token.read().unwrap().as_ref() {
            builder = builder.bearer_auth(token);
        }
        let response = builder
            .header(reqwest::header::RANGE, format!("bytes={start}-{end}"))
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            return Err(ApiError::Server {
                status: status.as_u16(),
                message: format!("audio chunk fetch failed ({status})"),
            });
        }
        let headers = response.headers();
        let content_type = headers
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        let content_range = headers
            .get(reqwest::header::CONTENT_RANGE)
            .and_then(|v| v.to_str().ok());
        // `Content-Range` start (206), else 0 (a 200 served the whole file from 0).
        let served_from = content_range
            .and_then(parse_content_range_start)
            .unwrap_or(0);
        // Total size from `Content-Range: bytes <s>-<e>/<total>` (206), else from
        // `Content-Length` (a 200 that ignored the Range = the whole file).
        let total = content_range
            .and_then(parse_content_range_total)
            .or_else(|| {
                headers
                    .get(reqwest::header::CONTENT_LENGTH)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.parse::<u64>().ok())
            });
        // Mis-offset response (a proxy ignored the `Range` and answered from
        // byte 0): return WITHOUT consuming the body. The caller discards this
        // chunk on the `served_from` check anyway — pre-fix, `bytes()` first
        // buffered the entire (often 100+ MB) file per in-flight chunk, ×
        // parallelism, just to throw it away.
        if served_from != start {
            return Ok(AudioChunk {
                bytes: Vec::new(),
                total,
                served_from,
                content_type,
            });
        }
        // In-range response: consume incrementally, CAPPED at the requested
        // span. A 200 that served the whole file from byte 0 (server ignored
        // the range END) passes the offset check when `start == 0` — reading
        // it whole would buffer the entire file; capping keeps this a chunk.
        // Fewer/truncated bytes are fine: callers treat short chunks as
        // ordinary partial progress and re-request the remainder.
        let requested = (end.saturating_sub(start) as usize).saturating_add(1);
        let mut body = response.bytes_stream();
        let mut bytes: Vec<u8> = Vec::with_capacity(requested.min(16 * 1024 * 1024));
        while bytes.len() < requested {
            match body.next().await {
                Some(piece) => {
                    let piece = piece?;
                    let take = piece.len().min(requested - bytes.len());
                    bytes.extend_from_slice(&piece[..take]);
                    if take < piece.len() {
                        break; // requested span filled mid-piece; drop the rest
                    }
                }
                None => break,
            }
        }
        Ok(AudioChunk {
            bytes,
            total,
            served_from,
            content_type,
        })
    }

    /// GET /episodes/{id}/audio as a STREAM — the "no chunking" download mode: one
    /// request, but the body is consumed piece-by-piece so the whole (often 100+ MB)
    /// file is never buffered in memory. Sends an open-ended `Range: bytes={start}-`
    /// so a resume continues from `start` and a fresh download (`start == 0`) gets
    /// the whole file without picking an upper bound. The returned [`AudioStream`]
    /// carries `total` + `content_type` from the headers and `served_from` (the
    /// offset the body starts at) so the caller can detect a server that ignored the
    /// range (answered `200` from byte 0) and restart instead of appending.
    pub async fn download_audio_stream(
        &self,
        id: i32,
        start: u64,
    ) -> Result<AudioStream, ApiError> {
        let url = format!("{}/episodes/{}/audio", self.base, id);
        let mut builder = self.http.get(&url);
        if let Some(token) = self.token.read().unwrap().as_ref() {
            builder = builder.bearer_auth(token);
        }
        let response = builder
            .header(reqwest::header::RANGE, format!("bytes={start}-"))
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            return Err(ApiError::Server {
                status: status.as_u16(),
                message: format!("audio stream fetch failed ({status})"),
            });
        }
        let headers = response.headers();
        let content_type = headers
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        let content_range = headers
            .get(reqwest::header::CONTENT_RANGE)
            .and_then(|v| v.to_str().ok());
        // `Content-Range` start (206), else 0 (a 200 served the whole file from 0).
        let served_from = content_range
            .and_then(parse_content_range_start)
            .unwrap_or(0);
        // Full size from `Content-Range` (206) — NOT `Content-Length`, which on a 206
        // is only the partial length. `Content-Length` is the fallback for a 200.
        let total = content_range
            .and_then(parse_content_range_total)
            .or_else(|| {
                headers
                    .get(reqwest::header::CONTENT_LENGTH)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.parse::<u64>().ok())
            });
        let body = response
            .bytes_stream()
            .map(|item| item.map(|b| b.to_vec()).map_err(ApiError::from));
        Ok(AudioStream {
            total,
            served_from,
            content_type,
            body: Box::pin(body),
        })
    }

    /// GET an arbitrary **media** path under `/api/v1` (`episodes/{id}/art`,
    /// `episodes/{id}/audio`, …) with the bearer token, optionally forwarding a
    /// `Range` header verbatim. Backs the native webview's authenticated media
    /// proxy: `<img>`/`<audio>` inside the webview can attach neither the
    /// bearer header nor the `auth_media` cookie (login happened in reqwest,
    /// not the webview), so `ui-state`'s `WebviewMediaBridge` fetches through
    /// this instead. Non-2xx statuses are returned in the response (not as
    /// errors) so the proxy forwards them verbatim; only transport/build
    /// failures error. The caller validates `path` against an allowlist.
    #[cfg(not(target_arch = "wasm32"))]
    pub async fn fetch_media_raw(
        &self,
        path: &str,
        range: Option<&str>,
    ) -> Result<crate::audio::RawMediaResponse, ApiError> {
        let url = format!("{}/{path}", self.base);
        let mut builder = self.http.get(&url);
        if let Some(token) = self.token.read().unwrap().as_ref() {
            builder = builder.bearer_auth(token);
        }
        if let Some(range) = range {
            builder = builder.header(reqwest::header::RANGE, range);
        }
        let response = builder.send().await?;
        let status = response.status().as_u16();
        let text_header = |name: reqwest::header::HeaderName| {
            response
                .headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string)
        };
        let content_type = text_header(reqwest::header::CONTENT_TYPE);
        let content_range = text_header(reqwest::header::CONTENT_RANGE);
        let cache_control = text_header(reqwest::header::CACHE_CONTROL);
        let etag = text_header(reqwest::header::ETAG);
        let bytes = response.bytes().await?.to_vec();
        Ok(crate::audio::RawMediaResponse {
            status,
            content_type,
            content_range,
            cache_control,
            etag,
            bytes,
        })
    }

    /// POST /episodes/{id}/download — trigger a server-side download of the
    /// episode's audio. Returns once accepted (202); the server fetches in the
    /// background and the new `download_status` shows up on the next list/get.
    pub async fn trigger_download(&self, id: i32) -> Result<(), ApiError> {
        let url = format!("{}/episodes/{}/download", self.base, id);
        let response = self.authed_post(url, String::new()).await?;
        self.handle_empty_response(response).await
    }

    /// DELETE /episodes/{id}/download — remove the server's downloaded copy and
    /// reset the episode to `NotDownloaded`. The status flip shows up on the next
    /// list/get pull.
    pub async fn remove_server_download(&self, id: i32) -> Result<(), ApiError> {
        let url = format!("{}/episodes/{}/download", self.base, id);
        let response = self.authed_delete(url).await?;
        self.handle_empty_response(response).await
    }

    /// POST /episodes/download/bulk — trigger a server-side download for many
    /// episodes in one request. The server filters out ids the caller isn't
    /// authorized for; returns 202 (background fetches), like the single route.
    pub async fn trigger_download_bulk(&self, episode_ids: Vec<i32>) -> Result<(), ApiError> {
        let body =
            halogen_wire::RequestData::<_, ()>::from_data(halogen_wire::EpisodeBulkActionData {
                episode_ids,
            });
        let body_str = serde_json::to_string(&body).map_err(|e| ApiError::Decode(e.to_string()))?;
        let url = format!("{}/episodes/download/bulk", self.base);
        let response = self.authed_post(url, body_str).await?;
        self.handle_empty_response(response).await
    }

    /// DELETE /episodes/download/bulk — remove the server's downloaded copy for
    /// many episodes in one request (unauthorized ids filtered out server-side).
    pub async fn remove_server_download_bulk(&self, episode_ids: Vec<i32>) -> Result<(), ApiError> {
        let body =
            halogen_wire::RequestData::<_, ()>::from_data(halogen_wire::EpisodeBulkActionData {
                episode_ids,
            });
        let body_str = serde_json::to_string(&body).map_err(|e| ApiError::Decode(e.to_string()))?;
        let url = format!("{}/episodes/download/bulk", self.base);
        let response = self.authed_delete_with_body(url, body_str).await?;
        self.handle_empty_response(response).await
    }

    /// GET /episodes/{id}/download-progress — a live snapshot of the server's
    /// in-flight fetch of this episode (bytes + percent), or `None` when nothing is
    /// running for it. The endpoint 404s before the server starts and once the
    /// download reaches a terminal state (the durable outcome then lives on
    /// `episode.download_status`), so `Ok(None)` covers both "not started" and
    /// "finished" — the caller disambiguates by whether it had seen progress.
    pub async fn get_download_progress(
        &self,
        id: i32,
    ) -> Result<Option<halogen_wire::DownloadProgressData>, ApiError> {
        let url = format!("{}/episodes/{}/download-progress", self.base, id);
        let response = self.authed_get(url).await?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let resp: Page<halogen_wire::DownloadProgressData> = self.handle_response(response).await?;
        Ok(Some(resp.data))
    }

    /// PUT /episodes/{id}
    pub async fn update_episode(
        &self,
        id: i32,
        data: EpisodeUpdateData,
    ) -> Result<EpisodeData, ApiError> {
        let body = halogen_wire::RequestData::<_, ()>::from_data(data);
        let body_str = serde_json::to_string(&body).map_err(|e| ApiError::Decode(e.to_string()))?;
        let url = format!("{}/episodes/{}", self.base, id);
        let response = self.authed_put(url, body_str).await?;
        let resp: Page<EpisodeData> = self.handle_response(response).await?;
        Ok(resp.data)
    }

    // ── Playbacks ────────────────────────────────────────────────────────

    /// GET /playbacks — list playbacks with pagination.
    pub async fn list_playbacks(
        &self,
        params: PlaybackListParams,
    ) -> Result<Page<Vec<PlaybackData>>, ApiError> {
        let qs = serde_qs::to_string(&params).map_err(|e| ApiError::Decode(e.to_string()))?;
        let url = build_url(&self.base, "/playbacks", &qs);
        let response = self.authed_get(url).await?;
        self.handle_response(response).await
    }

    /// POST /playbacks — upsert a playback.
    pub async fn upsert_playback(&self, data: PlaybackStoreData) -> Result<PlaybackData, ApiError> {
        let body = halogen_wire::RequestData::<_, ()>::from_data(data);
        let body_str = serde_json::to_string(&body).map_err(|e| ApiError::Decode(e.to_string()))?;
        let url = format!("{}{}", self.base, "/playbacks");
        let response = self.authed_post(url, body_str).await?;
        let resp: Page<PlaybackData> = self.handle_response(response).await?;
        Ok(resp.data)
    }

    /// DELETE /playbacks/{id}
    pub async fn delete_playback(&self, id: i32) -> Result<(), ApiError> {
        let url = format!("{}/playbacks/{}", self.base, id);
        let response = self.authed_delete(url).await?;
        self.handle_empty_response(response).await?;
        Ok(())
    }

    // ── Playlists ────────────────────────────────────────────────────────

    /// GET /playlists — list playlists with pagination.
    pub async fn list_playlists(
        &self,
        params: halogen_wire::DefaultListParams<PlaylistInclude>,
    ) -> Result<Page<Vec<PlaylistData>>, ApiError> {
        let qs = serde_qs::to_string(&params).map_err(|e| ApiError::Decode(e.to_string()))?;
        let url = build_url(&self.base, "/playlists", &qs);
        let response = self.authed_get(url).await?;
        self.handle_response(response).await
    }

    /// GET /playlists/{id}
    pub async fn get_playlist(&self, id: i32) -> Result<PlaylistData, ApiError> {
        let url = format!("{}/playlists/{}", self.base, id);
        let response = self.authed_get(url).await?;
        let resp: Page<PlaylistData> = self.handle_response(response).await?;
        Ok(resp.data)
    }

    /// GET /playlists/default — the user's queue (default playlist) with its
    /// ordered episode ids, or `None` when no default exists. Decoupled from the
    /// paged playlist list so the client always knows its queue.
    pub async fn get_default_playlist(&self) -> Result<Option<PlaylistData>, ApiError> {
        let url = format!("{}/playlists/default", self.base);
        let response = self.authed_get(url).await?;
        let resp: Page<DefaultPlaylistData> = self.handle_response(response).await?;
        Ok(resp.data.playlist)
    }

    /// POST /playlists
    pub async fn create_playlist(&self, data: PlaylistStoreData) -> Result<PlaylistData, ApiError> {
        let body = halogen_wire::RequestData::<_, ()>::from_data(data);
        let body_str = serde_json::to_string(&body).map_err(|e| ApiError::Decode(e.to_string()))?;
        let url = format!("{}{}", self.base, "/playlists");
        let response = self.authed_post(url, body_str).await?;
        let resp: Page<PlaylistData> = self.handle_response(response).await?;
        Ok(resp.data)
    }

    /// PUT /playlists/{id}
    pub async fn update_playlist(
        &self,
        id: i32,
        data: PlaylistUpdateData,
    ) -> Result<PlaylistData, ApiError> {
        let body = halogen_wire::RequestData::<_, ()>::from_data(data);
        let body_str = serde_json::to_string(&body).map_err(|e| ApiError::Decode(e.to_string()))?;
        let url = format!("{}/playlists/{}", self.base, id);
        let response = self.authed_put(url, body_str).await?;
        let resp: Page<PlaylistData> = self.handle_response(response).await?;
        Ok(resp.data)
    }

    /// DELETE /playlists/{id}
    pub async fn delete_playlist(&self, id: i32) -> Result<(), ApiError> {
        let url = format!("{}/playlists/{}", self.base, id);
        let response = self.authed_delete(url).await?;
        self.handle_empty_response(response).await?;
        Ok(())
    }

    /// POST /playlists/{playlist_id}/episodes/{episode_id}
    ///
    /// `position` is the insert index: `Some(0)` = front, `None` = append (default).
    pub async fn add_episode(
        &self,
        playlist_id: i32,
        episode_id: i32,
        position: Option<i32>,
    ) -> Result<EpisodePlaylistData, ApiError> {
        let body =
            halogen_wire::RequestData::<_, ()>::from_data(EpisodePlaylistStoreData { position });
        let body_str = serde_json::to_string(&body).map_err(|e| ApiError::Decode(e.to_string()))?;
        let url = format!(
            "{}/playlists/{}/episodes/{}",
            self.base, playlist_id, episode_id
        );
        let response = self.authed_post(url, body_str).await?;
        let resp: Page<EpisodePlaylistData> = self.handle_response(response).await?;
        Ok(resp.data)
    }

    /// DELETE /playlists/{playlist_id}/episodes/{episode_id}
    pub async fn remove_episode(&self, playlist_id: i32, episode_id: i32) -> Result<(), ApiError> {
        let url = format!(
            "{}/playlists/{}/episodes/{}",
            self.base, playlist_id, episode_id
        );
        let response = self.authed_delete(url).await?;
        self.handle_empty_response(response).await?;
        Ok(())
    }

    /// POST /playlists/{playlist_id}/episodes/bulk — add many episodes to one
    /// playlist in a single request (append). The server owner-guards the playlist
    /// and filters out ids the caller isn't authorized for; lenient per id.
    pub async fn add_episodes_bulk(
        &self,
        playlist_id: i32,
        episode_ids: Vec<i32>,
    ) -> Result<(), ApiError> {
        let body =
            halogen_wire::RequestData::<_, ()>::from_data(EpisodePlaylistBulkData { episode_ids });
        let body_str = serde_json::to_string(&body).map_err(|e| ApiError::Decode(e.to_string()))?;
        let url = format!("{}/playlists/{}/episodes/bulk", self.base, playlist_id);
        let response = self.authed_post(url, body_str).await?;
        self.handle_empty_response(response).await
    }

    /// DELETE /playlists/{playlist_id}/episodes/bulk — remove many episodes from one
    /// playlist in a single request (non-members filtered/skipped server-side).
    pub async fn remove_episodes_bulk(
        &self,
        playlist_id: i32,
        episode_ids: Vec<i32>,
    ) -> Result<(), ApiError> {
        let body =
            halogen_wire::RequestData::<_, ()>::from_data(EpisodePlaylistBulkData { episode_ids });
        let body_str = serde_json::to_string(&body).map_err(|e| ApiError::Decode(e.to_string()))?;
        let url = format!("{}/playlists/{}/episodes/bulk", self.base, playlist_id);
        let response = self.authed_delete_with_body(url, body_str).await?;
        self.handle_empty_response(response).await
    }

    /// POST /playlists/{playlist_id}/episodes/{episode_id}/move — reorder an
    /// episode to target index `to`; the server rewrites positions 0..n.
    pub async fn move_episode(
        &self,
        playlist_id: i32,
        episode_id: i32,
        to: i32,
    ) -> Result<(), ApiError> {
        let body = halogen_wire::RequestData::<_, ()>::from_data(EpisodePlaylistMoveData { to });
        let body_str = serde_json::to_string(&body).map_err(|e| ApiError::Decode(e.to_string()))?;
        let url = format!(
            "{}/playlists/{}/episodes/{}/move",
            self.base, playlist_id, episode_id
        );
        let response = self.authed_post(url, body_str).await?;
        self.handle_empty_response(response).await?;
        Ok(())
    }

    /// POST /playlists/{id}/move — reorder a playlist to target index `to`; the
    /// server rewrites every playlist's `position` 0..n.
    pub async fn move_playlist(&self, playlist_id: i32, to: i32) -> Result<(), ApiError> {
        let body =
            halogen_wire::RequestData::<_, ()>::from_data(halogen_wire::PlaylistMoveData { to });
        let body_str = serde_json::to_string(&body).map_err(|e| ApiError::Decode(e.to_string()))?;
        let url = format!("{}/playlists/{}/move", self.base, playlist_id);
        let response = self.authed_post(url, body_str).await?;
        self.handle_empty_response(response).await?;
        Ok(())
    }

    /// POST /playlists/{id}/reorder-by — smart-reorder a playlist's episodes by
    /// `field`/`direction`, baking the order into the `Custom` (position) sequence.
    pub async fn reorder_playlist(
        &self,
        playlist_id: i32,
        field: halogen_wire::PlaylistReorderField,
        direction: halogen_wire::OrderDirection,
    ) -> Result<(), ApiError> {
        let body =
            halogen_wire::RequestData::<_, ()>::from_data(halogen_wire::PlaylistReorderData {
                field,
                direction,
            });
        let body_str = serde_json::to_string(&body).map_err(|e| ApiError::Decode(e.to_string()))?;
        let url = format!("{}/playlists/{}/reorder-by", self.base, playlist_id);
        let response = self.authed_post(url, body_str).await?;
        self.handle_empty_response(response).await?;
        Ok(())
    }

    /// GET /episodes/{id}/playlists — the caller's playlists that contain this
    /// episode (picker pre-selection).
    pub async fn list_episode_playlists(
        &self,
        episode_id: i32,
        params: halogen_wire::DefaultListParams<PlaylistInclude>,
    ) -> Result<Page<Vec<PlaylistData>>, ApiError> {
        let qs = serde_qs::to_string(&params).map_err(|e| ApiError::Decode(e.to_string()))?;
        let url = build_url(
            &self.base,
            &format!("/episodes/{}/playlists", episode_id),
            &qs,
        );
        let response = self.authed_get(url).await?;
        self.handle_response(response).await
    }

    /// GET /playlists/{id}/episodes — list episodes in a playlist with pagination.
    pub async fn list_playlist_episodes(
        &self,
        playlist_id: i32,
        params: halogen_wire::DefaultListParams<EpisodeInclude>,
    ) -> Result<Page<Vec<EpisodeData>>, ApiError> {
        let qs = serde_qs::to_string(&params).map_err(|e| ApiError::Decode(e.to_string()))?;
        let url = build_url(
            &self.base,
            &format!("/playlists/{}/episodes", playlist_id),
            &qs,
        );
        let response = self.authed_get(url).await?;
        self.handle_response(response).await
    }

    // ── Podcast Configs ──────────────────────────────────────────────────

    /// GET /podcast-configs/{id}
    pub async fn get_podcast_config(&self, id: i32) -> Result<PodcastConfigData, ApiError> {
        let url = format!("{}/podcast-configs/{}", self.base, id);
        let response = self.authed_get(url).await?;
        let resp: Page<PodcastConfigData> = self.handle_response(response).await?;
        Ok(resp.data)
    }

    /// PUT /podcast-configs/{id}
    pub async fn update_podcast_config(
        &self,
        id: i32,
        data: PodcastConfigUpdateData,
    ) -> Result<PodcastConfigData, ApiError> {
        let body = halogen_wire::RequestData::<_, ()>::from_data(data);
        let body_str = serde_json::to_string(&body).map_err(|e| ApiError::Decode(e.to_string()))?;
        let url = format!("{}/podcast-configs/{}", self.base, id);
        let response = self.authed_put(url, body_str).await?;
        let resp: Page<PodcastConfigData> = self.handle_response(response).await?;
        Ok(resp.data)
    }

    /// POST /podcasts/{id}/config — create a config AND link it to the podcast in
    /// one atomic call. This is the only way to create a config (there is no
    /// standalone, unlinked create). Returns the new config.
    pub async fn create_podcast_config_for(
        &self,
        podcast_id: i32,
        data: PodcastConfigStoreData,
    ) -> Result<PodcastConfigData, ApiError> {
        let body = halogen_wire::RequestData::<_, ()>::from_data(data);
        let body_str = serde_json::to_string(&body).map_err(|e| ApiError::Decode(e.to_string()))?;
        let url = format!("{}/podcasts/{}/config", self.base, podcast_id);
        let response = self.authed_post(url, body_str).await?;
        let resp: Page<PodcastConfigData> = self.handle_response(response).await?;
        Ok(resp.data)
    }

    /// DELETE /podcasts/{id}/config — unlink + delete the podcast's config (revert
    /// to global defaults), atomically.
    pub async fn remove_podcast_config_for(&self, podcast_id: i32) -> Result<(), ApiError> {
        let url = format!("{}/podcasts/{}/config", self.base, podcast_id);
        let response = self.authed_delete(url).await?;
        self.handle_empty_response(response).await?;
        Ok(())
    }

    // ── Podcast Auto-Playlists ───────────────────────────────────────────

    /// GET /podcasts/{id}/auto-playlists — the playlists this podcast auto-adds
    /// new episodes to. Stale links (playlist since deleted) are filtered server-
    /// side, so every returned id is a live playlist.
    pub async fn get_podcast_auto_playlists(
        &self,
        podcast_id: i32,
    ) -> Result<Vec<PodcastAutoPlaylistData>, ApiError> {
        let url = format!("{}/podcasts/{}/auto-playlists", self.base, podcast_id);
        let response = self.authed_get(url).await?;
        let resp: Page<Vec<PodcastAutoPlaylistData>> = self.handle_response(response).await?;
        Ok(resp.data)
    }

    /// PUT /podcasts/{id}/auto-playlists — replace the podcast's full set of
    /// auto-add playlists with `playlist_ids` (idempotent; unknown ids are
    /// dropped server-side). `add_to_start` is the podcast's insert-position
    /// override, stamped on every link: `Some(true)` = start of the playlists,
    /// `Some(false)` = end, `None` = follow the server-wide default. Returns
    /// the resulting set.
    pub async fn set_podcast_auto_playlists(
        &self,
        podcast_id: i32,
        playlist_ids: Vec<i32>,
        add_to_start: Option<bool>,
    ) -> Result<Vec<PodcastAutoPlaylistData>, ApiError> {
        let body = halogen_wire::RequestData::<_, ()>::from_data(PodcastAutoPlaylistSetData {
            playlist_ids,
            add_to_start,
        });
        let body_str = serde_json::to_string(&body).map_err(|e| ApiError::Decode(e.to_string()))?;
        let url = format!("{}/podcasts/{}/auto-playlists", self.base, podcast_id);
        let response = self.authed_put(url, body_str).await?;
        let resp: Page<Vec<PodcastAutoPlaylistData>> = self.handle_response(response).await?;
        Ok(resp.data)
    }

    // ── Users ────────────────────────────────────────────────────────────

    /// GET /admin/users — list users with pagination. **Admin only.**
    pub async fn list_users(
        &self,
        params: halogen_wire::DefaultListParams<halogen_wire::NoInclude>,
    ) -> Result<Page<Vec<UserData>>, ApiError> {
        let qs = serde_qs::to_string(&params).map_err(|e| ApiError::Decode(e.to_string()))?;
        let url = build_url(&self.base, "/admin/users", &qs);
        let response = self.authed_get(url).await?;
        self.handle_response(response).await
    }

    // ── Polling control (admin only) ─────────────────────────────────────

    /// GET /status — whether the background polling service is running.
    pub async fn poll_status(&self) -> Result<PollingStatusData, ApiError> {
        let url = format!("{}/status", self.base);
        let response = self.authed_get(url).await?;
        self.handle_single_response(response).await
    }

    /// POST /admin/poll — trigger a one-off feed sync immediately. **Admin only.**
    pub async fn poll_now(&self) -> Result<PollingOperationData, ApiError> {
        let url = format!("{}/admin/poll", self.base);
        let response = self.authed_post(url, String::new()).await?;
        self.handle_single_response(response).await
    }

    /// POST /admin/poll-job — start an on-demand poll job and get its id back
    /// immediately. `podcast_id` scopes the run to one feed (`None` = all). Poll
    /// [`Self::get_poll_job`] for progress. **Admin only.**
    pub async fn start_poll_job(
        &self,
        podcast_id: Option<i32>,
    ) -> Result<PollJobStartData, ApiError> {
        let qs = match podcast_id {
            Some(id) => format!("podcast_id={id}"),
            None => String::new(),
        };
        let url = build_url(&self.base, "/admin/poll-job", &qs);
        let response = self.authed_post(url, String::new()).await?;
        self.handle_single_response(response).await
    }

    /// GET /admin/poll-job/{id} — current snapshot of a poll job. **Admin only.**
    pub async fn get_poll_job(&self, id: u64) -> Result<PollJobData, ApiError> {
        let url = format!("{}/admin/poll-job/{}", self.base, id);
        let response = self.authed_get(url).await?;
        self.handle_single_response(response).await
    }

    /// GET /admin/poll-jobs — recent poll jobs, newest first (capped). **Admin only.**
    pub async fn list_poll_jobs(&self) -> Result<Vec<PollJobData>, ApiError> {
        let url = format!("{}/admin/poll-jobs", self.base);
        let response = self.authed_get(url).await?;
        let resp: Page<Vec<PollJobData>> = self.handle_response(response).await?;
        Ok(resp.data)
    }

    /// POST /admin/start — start the background polling service. **Admin only.**
    pub async fn start_polling(&self) -> Result<PollingOperationData, ApiError> {
        let url = format!("{}/admin/start", self.base);
        let response = self.authed_post(url, String::new()).await?;
        self.handle_single_response(response).await
    }

    /// POST /admin/stop — stop the background polling service. **Admin only.**
    pub async fn stop_polling(&self) -> Result<PollingOperationData, ApiError> {
        let url = format!("{}/admin/stop", self.base);
        let response = self.authed_post(url, String::new()).await?;
        self.handle_single_response(response).await
    }

    /// GET /admin/config — the reconciled runtime config, minus secrets. **Admin only.**
    ///
    /// Returns a single sanitised [`ConfigData`] (not paginated), so it uses the
    /// single-object response handling like [`Self::health`].
    pub async fn get_config(&self) -> Result<ConfigData, ApiError> {
        let url = format!("{}/admin/config", self.base);
        let response = self.authed_get(url).await?;
        self.handle_single_response(response).await
    }

    /// GET /admin/config-overrides — the persisted overrides allowlist (only keys that
    /// are actually set). **Admin only.** Empty when none. Prepopulates the editor.
    pub async fn get_config_overrides(&self) -> Result<ConfigOverridesData, ApiError> {
        let url = format!("{}/admin/config-overrides", self.base);
        let response = self.authed_get(url).await?;
        self.handle_single_response(response).await
    }

    /// POST /admin/config-overrides — **replace** the overrides with `data` verbatim.
    /// **Admin only.** Any allowlisted key omitted from `data` is deleted. Does
    /// not restart — apply via [`Self::restart_server`]. Returns the written set.
    pub async fn set_config_overrides(
        &self,
        data: ConfigOverridesData,
    ) -> Result<ConfigOverridesData, ApiError> {
        let body = halogen_wire::RequestData::<_, ()>::from_data(data);
        let body_str = serde_json::to_string(&body).map_err(|e| ApiError::Decode(e.to_string()))?;
        let url = format!("{}/admin/config-overrides", self.base);
        let response = self.authed_post(url, body_str).await?;
        self.handle_single_response(response).await
    }

    /// DELETE /admin/config-overrides — clear ALL overrides. **Admin only.** Does not
    /// restart.
    pub async fn delete_config_overrides(&self) -> Result<(), ApiError> {
        let url = format!("{}/admin/config-overrides", self.base);
        let response = self.authed_delete(url).await?;
        self.handle_empty_response(response).await
    }

    /// POST /admin/server/restart — request a graceful re-exec so written overrides take
    /// effect. **Admin only.** The connection drops while the process restarts.
    pub async fn restart_server(&self) -> Result<(), ApiError> {
        let url = format!("{}/admin/server/restart", self.base);
        let response = self.authed_post(url, String::new()).await?;
        // The handler echoes a small status object; we only care about success.
        let _: PollingOperationData = self.handle_single_response(response).await?;
        Ok(())
    }

    /// POST /admin/users — create a user. **Admin only.** Powers the embedded
    /// server's add-account flow (the app generates + stores the password for
    /// silent login) and ordinary provisioning.
    pub async fn create_user(&self, data: UserStoreData) -> Result<UserData, ApiError> {
        let body = halogen_wire::RequestData::<_, ()>::from_data(data);
        let body_str = serde_json::to_string(&body).map_err(|e| ApiError::Decode(e.to_string()))?;
        let url = format!("{}/admin/users", self.base);
        let response = self.authed_post(url, body_str).await?;
        let resp: Page<UserData> = self.handle_response(response).await?;
        Ok(resp.data)
    }

    /// GET /admin/db/export — the gzipped, scrubbed database export (no
    /// password hashes, nothing marked downloaded, no operational history).
    /// **Admin only.** Returns the raw `.db.gz` bytes for the caller to save.
    pub async fn export_db(&self) -> Result<Vec<u8>, ApiError> {
        let url = format!("{}/admin/db/export", self.base);
        let response = self.authed_get(url).await?;
        let status = response.status();
        if !status.is_success() {
            let bytes = response.bytes().await.unwrap_or_default();
            // Either arm of `parse_error_body` is an error to surface.
            return match parse_error_body(status.as_u16(), &bytes) {
                Ok(e) | Err(e) => Err(e),
            };
        }
        let bytes = response.bytes().await?;
        Ok(bytes.to_vec())
    }

    /// POST /admin/db/import — merge an export (gzipped or raw SQLite) into
    /// the server's database. **Admin only.** Returns the per-entity summary
    /// (see the DTO for the merge rules).
    pub async fn import_db(&self, bytes: Vec<u8>) -> Result<DbImportSummaryData, ApiError> {
        let url = format!("{}/admin/db/import", self.base);
        let mut builder = self.http.post(&url);
        if let Some(token) = self.token.read().unwrap().as_ref() {
            builder = builder.bearer_auth(token);
        }
        let response = builder
            .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
            .body(bytes)
            .send()
            .await
            .map_err(ApiError::from)?;
        self.handle_single_response(response).await
    }

    /// GET /admin/server-errors — the persisted failure histories (RSS sync per
    /// podcast, media download per episode), newest first. **Admin only.**
    pub async fn get_server_errors(&self) -> Result<ServerErrorsData, ApiError> {
        let url = format!("{}/admin/server-errors", self.base);
        let response = self.authed_get(url).await?;
        self.handle_single_response(response).await
    }

    /// GET /admin/server-logs — tail of the server's log file. **Admin only.**
    /// `lines` caps how many trailing lines come back (server default/caps apply).
    pub async fn get_server_logs(&self, lines: Option<usize>) -> Result<ServerLogsData, ApiError> {
        let qs = match lines {
            Some(n) => format!("lines={n}"),
            None => String::new(),
        };
        let url = build_url(&self.base, "/admin/server-logs", &qs);
        let response = self.authed_get(url).await?;
        self.handle_single_response(response).await
    }

    /// POST /admin/opml/import — import podcasts from OPML XML. **Admin only.**
    pub async fn import_opml(
        &self,
        data: OpmlImportData,
    ) -> Result<OpmlImportResultData, ApiError> {
        let body = halogen_wire::RequestData::<_, ()>::from_data(data);
        let body_str = serde_json::to_string(&body).map_err(|e| ApiError::Decode(e.to_string()))?;
        let url = format!("{}/admin/opml/import", self.base);
        let response = self.authed_post(url, body_str).await?;
        self.handle_single_response(response).await
    }

    /// GET /admin/opml/export — the current subscriptions as OPML XML. **Admin only.**
    pub async fn export_opml(&self) -> Result<OpmlExportData, ApiError> {
        let url = format!("{}/admin/opml/export", self.base);
        let response = self.authed_get(url).await?;
        self.handle_single_response(response).await
    }

    /// GET /discover/search?q=...&providers[]=itunes... — federated provider search.
    ///
    /// Online-only. Returns a single [`DiscoverSearchData`] (never paginated)
    /// carrying merged results plus a per-provider error list, so a provider
    /// failing yields partial results rather than an error. `serde_qs` urlencodes
    /// `q` and the `providers[]` filter.
    pub async fn discover_search(
        &self,
        params: DiscoverSearchParams,
    ) -> Result<DiscoverSearchData, ApiError> {
        let qs = serde_qs::to_string(&params).map_err(|e| ApiError::Decode(e.to_string()))?;
        let url = build_url(&self.base, "/discover/search", &qs);
        let response = self.authed_get(url).await?;
        self.handle_single_response(response).await
    }

    /// GET /discover/providers — which providers exist and are currently available
    /// (so the UI knows which toggle chips to render).
    pub async fn discover_providers(&self) -> Result<DiscoverProvidersData, ApiError> {
        let url = format!("{}/discover/providers", self.base);
        let response = self.authed_get(url).await?;
        self.handle_single_response(response).await
    }

    /// GET /users/{id}
    pub async fn get_user(&self, id: i32) -> Result<UserData, ApiError> {
        let url = format!("{}/users/{}", self.base, id);
        let response = self.authed_get(url).await?;
        let resp: Page<UserData> = self.handle_response(response).await?;
        Ok(resp.data)
    }

    /// PUT /users/{id}
    pub async fn update_user(&self, id: i32, data: UserUpdateData) -> Result<UserData, ApiError> {
        let body = halogen_wire::RequestData::<_, ()>::from_data(data);
        let body_str = serde_json::to_string(&body).map_err(|e| ApiError::Decode(e.to_string()))?;
        let url = format!("{}/users/{}", self.base, id);
        let response = self.authed_put(url, body_str).await?;
        let resp: Page<UserData> = self.handle_response(response).await?;
        Ok(resp.data)
    }

    /// POST /auth/password — change the authenticated user's own password. The
    /// account is taken from the bearer token server-side (never the body), so this
    /// only ever changes the caller's own password. Online-only (no outbox).
    pub async fn change_password(&self, data: PasswordChangeData) -> Result<(), ApiError> {
        let body = halogen_wire::RequestData::<_, ()>::from_data(data);
        let body_str = serde_json::to_string(&body).map_err(|e| ApiError::Decode(e.to_string()))?;
        let url = format!("{}/auth/password", self.base);
        let response = self.authed_post(url, body_str).await?;
        self.handle_empty_response(response).await
    }

    /// DELETE /admin/users/{id} — remove a user. **Admin only.**
    pub async fn delete_user(&self, id: i32) -> Result<(), ApiError> {
        let url = format!("{}/admin/users/{}", self.base, id);
        let response = self.authed_delete(url).await?;
        self.handle_empty_response(response).await?;
        Ok(())
    }

    // ── Auth helpers (private) ───────────────────────────────────────────

    async fn authed_get(&self, url: String) -> Result<reqwest::Response, ApiError> {
        let mut builder = self.http.get(&url);
        if let Some(token) = self.token.read().unwrap().as_ref() {
            builder = builder.bearer_auth(token);
        }
        builder.send().await.map_err(ApiError::from)
    }

    async fn authed_post(&self, url: String, body: String) -> Result<reqwest::Response, ApiError> {
        let mut builder = self.http.post(&url);
        if let Some(token) = self.token.read().unwrap().as_ref() {
            builder = builder.bearer_auth(token);
        }
        builder
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body)
            .send()
            .await
            .map_err(ApiError::from)
    }

    async fn authed_put(&self, url: String, body: String) -> Result<reqwest::Response, ApiError> {
        let mut builder = self.http.put(&url);
        if let Some(token) = self.token.read().unwrap().as_ref() {
            builder = builder.bearer_auth(token);
        }
        builder
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body)
            .send()
            .await
            .map_err(ApiError::from)
    }

    async fn authed_delete(&self, url: String) -> Result<reqwest::Response, ApiError> {
        let mut builder = self.http.delete(&url);
        if let Some(token) = self.token.read().unwrap().as_ref() {
            builder = builder.bearer_auth(token);
        }
        builder.send().await.map_err(ApiError::from)
    }

    /// DELETE with a JSON body — needed by the bulk-remove endpoint (the plain
    /// `authed_delete` sends no body). Axum's `Body<T>` extractor reads the body on
    /// DELETE just like POST.
    async fn authed_delete_with_body(
        &self,
        url: String,
        body: String,
    ) -> Result<reqwest::Response, ApiError> {
        let mut builder = self.http.delete(&url);
        if let Some(token) = self.token.read().unwrap().as_ref() {
            builder = builder.bearer_auth(token);
        }
        builder
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body)
            .send()
            .await
            .map_err(ApiError::from)
    }

    // ── Response handling (private) ──────────────────────────────────────

    async fn handle_response<Res: DeserializeOwned + ResponsableData>(
        &self,
        response: reqwest::Response,
    ) -> Result<Page<Res>, ApiError> {
        let status = response.status();
        let bytes = response.bytes().await?;

        if !status.is_success() {
            if let Ok(api_err) = parse_error_body(status.as_u16(), &bytes) {
                return Err(api_err);
            }
            return Err(ApiError::Server {
                status: status.as_u16(),
                message: String::from_utf8_lossy(&bytes).to_string(),
            });
        }

        let resp: halogen_wire::ResponseData<Res> =
            serde_json::from_slice(&bytes).map_err(|e| ApiError::Decode(e.to_string()))?;

        let data = resp.data.ok_or(ApiError::Empty)?;
        Ok(Page {
            data,
            paginator: resp.paginator,
        })
    }

    async fn handle_single_response<Res: DeserializeOwned + ResponsableData>(
        &self,
        response: reqwest::Response,
    ) -> Result<Res, ApiError> {
        let status = response.status();
        let bytes = response.bytes().await?;

        if !status.is_success() {
            if let Ok(api_err) = parse_error_body(status.as_u16(), &bytes) {
                return Err(api_err);
            }
            return Err(ApiError::Server {
                status: status.as_u16(),
                message: String::from_utf8_lossy(&bytes).to_string(),
            });
        }

        let resp: halogen_wire::ResponseData<Res> =
            serde_json::from_slice(&bytes).map_err(|e| ApiError::Decode(e.to_string()))?;

        resp.data.ok_or(ApiError::Empty)
    }

    async fn handle_empty_response(&self, response: reqwest::Response) -> Result<(), ApiError> {
        let status = response.status();
        let bytes = response.bytes().await?;

        if !status.is_success() {
            if let Ok(api_err) = parse_error_body(status.as_u16(), &bytes) {
                return Err(api_err);
            }
            return Err(ApiError::Server {
                status: status.as_u16(),
                message: String::from_utf8_lossy(&bytes).to_string(),
            });
        }

        let _: halogen_wire::ResponseData<()> =
            serde_json::from_slice(&bytes).map_err(|e| ApiError::Decode(e.to_string()))?;
        Ok(())
    }
}

/// Build a URL with optional query string.
fn build_url(base: &str, path: &str, qs: &str) -> String {
    if qs.is_empty() {
        format!("{}{}", base, path)
    } else {
        format!("{}{}?{}", base, path, qs)
    }
}
