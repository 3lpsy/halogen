//! `WebviewMediaBridge` — the native webview's media plumbing: an in-process
//! **loopback HTTP server** for artwork and audio.
//!
//! Two native problems it solves:
//! - **Auth**: on web, `<img>`/`<audio>` authenticate via the `auth_media`
//!   cookie set at login. The native app is a wry webview that never saw the
//!   login (auth is a bearer token inside reqwest), so server media URLs would
//!   401. The bridge proxies them with the bearer attached.
//! - **Transport**: this CANNOT be a webview custom-scheme asset handler.
//!   Images load through the webview's own loader (where a handler works),
//!   but media elements go through the platform media stack — WebKitGTK's
//!   GStreamer on Linux, and similarly constrained stacks in the Android
//!   WebView — which can only fetch http(s) URLs. `<audio src>` on a custom
//!   scheme fails instantly with MEDIA_ERR_SRC_NOT_SUPPORTED (the flatpak's
//!   "audio error (code 4)"). Loopback http is the ecosystem-standard fix
//!   (Tauri does the same on Linux).
//!
//! The server binds `127.0.0.1:0` synchronously at first render (so the port
//! is known before any URL is built) and publishes its base —
//! `http://127.0.0.1:{port}/{nonce}` — via
//! `ui-appstate::media_url::set_local_media_base`, which every media URL
//! builder resolves against. Routes, both nonce-guarded (the nonce is a
//! per-process 128-bit secret so other local processes can't browse the
//! proxy):
//!
//! - **`/{nonce}/halogen-local-audio/{file}`** — device-downloaded audio
//!   straight from the media store's audio dir, with HTTP Range support so
//!   the `<audio>` element can seek. URL producer: the `webview` player
//!   backend (`MediaSource::Local` file path → file name).
//! - **`/{nonce}/halogen-media/{path}`** — authenticated proxy to
//!   `{server}/api/v1/{path}` for the allowlisted media endpoints (artwork +
//!   the streaming audio route), forwarding `Range` (bounded) and relaying
//!   status/headers/bytes. URL producers: `ui-appstate::media_url` (art) and
//!   `audio_url_for_episode` (streaming).
//!
//! The server task runs on the tokio runtime (dioxus desktop provides
//! a multi-thread one), so everything it touches is `Send`: the live
//! server/token pair is mirrored from the config signal into a process-global
//! snapshot by this provider. Server + snapshot are process-global and bind
//! once — an account switch remounts the provider, which just refreshes the
//! snapshot. On web (and renderless native tests) this component is a
//! pass-through.

use dioxus::prelude::*;

/// Provider component; see the module docs. Pass-through on non-webview
/// targets.
#[component]
pub fn WebviewMediaBridge(children: Element) -> Element {
    #[cfg(all(not(target_arch = "wasm32"), feature = "desktop"))]
    imp::install();
    rsx! {
        {children}
    }
}

// ── Pure request logic (unit-tested in the renderless native build; the
//    server below only compiles under the webview renderer features) ─────────

/// Bound on how many bytes one open-ended (`bytes=N-`) audio range request
/// serves/proxies. Keeps a whole 100+ MB episode from being buffered for one
/// response; the webview's media stack follows up with the next range.
#[cfg(not(target_arch = "wasm32"))]
const STREAM_CHUNK_BYTES: u64 = 8 * 1024 * 1024;

/// Parse a `Range` header of the forms `bytes=N-` / `bytes=N-M`. Suffix ranges
/// (`bytes=-N`) and multipart ranges return `None` — the caller then answers
/// with the full body (a `200`, which is a legal response to any `Range`).
#[cfg(not(target_arch = "wasm32"))]
#[cfg_attr(not(feature = "desktop"), allow(dead_code))]
fn parse_range(header: &str) -> Option<(u64, Option<u64>)> {
    let spec = header.trim().strip_prefix("bytes=")?;
    let (start, end) = spec.split_once('-')?;
    let start: u64 = start.trim().parse().ok()?;
    let end = end.trim();
    if end.is_empty() {
        return Some((start, None));
    }
    let end: u64 = end.parse().ok()?;
    (end >= start).then_some((start, Some(end)))
}

/// Whether a proxied path is one of the media endpoints the bridge fronts.
/// Anything else (arbitrary API paths, traversal attempts) is refused — the
/// proxy must never become a generic authenticated tunnel.
#[cfg(not(target_arch = "wasm32"))]
#[cfg_attr(not(feature = "desktop"), allow(dead_code))]
fn allowed_media_path(rest: &str) -> bool {
    let parts: Vec<&str> = rest.split('/').collect();
    let id_ok = |id: &str| id.parse::<i32>().is_ok();
    match parts.as_slice() {
        ["episodes", id, "art"] | ["episodes", id, "art", "small"] => id_ok(id),
        ["podcasts", id, "art"] | ["podcasts", id, "art", "small"] => id_ok(id),
        ["episodes", id, "audio"] => id_ok(id),
        _ => false,
    }
}

/// Whether a local-audio file name is safe to join under the audio dir: the
/// plain `<episode>.<ext>` names the media store writes, nothing that can
/// traverse (`/`, `\`, `..`) or hide (`.` prefix).
#[cfg(not(target_arch = "wasm32"))]
#[cfg_attr(not(feature = "desktop"), allow(dead_code))]
fn safe_audio_file_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// Content type for a stored audio file name — the inverse of the media
/// store's `ext_for` MIME→extension mapping.
#[cfg(not(target_arch = "wasm32"))]
#[cfg_attr(not(feature = "desktop"), allow(dead_code))]
fn mime_for_name(name: &str) -> &'static str {
    match name.rsplit_once('.').map(|(_, ext)| ext) {
        Some("mp3") => "audio/mpeg",
        Some("m4a") => "audio/mp4",
        Some("ogg") => "audio/ogg",
        Some("opus") => "audio/opus",
        Some("wav") => "audio/wav",
        Some("flac") => "audio/flac",
        _ => "application/octet-stream",
    }
}

/// Clamp a parsed range against the file length and the chunk cap — EXPLICIT
/// ends too, not just open-ended ones: the range body is buffered in one `Vec`,
/// so an explicit `bytes=0-<huge>` from a media stack (or anything else in the
/// webview) would otherwise hold the entire 100+ MB file in RAM. Serving fewer
/// bytes than asked with an accurate `Content-Range` is valid 206 behavior;
/// players simply follow up. `None` = unsatisfiable (start past EOF → 416).
#[cfg(not(target_arch = "wasm32"))]
#[cfg_attr(not(feature = "desktop"), allow(dead_code))]
fn clamp_range(start: u64, end: Option<u64>, len: u64) -> Option<(u64, u64)> {
    if start >= len {
        return None;
    }
    let cap_end = start + STREAM_CHUNK_BYTES - 1;
    let end = end.map_or(cap_end, |e| e.min(cap_end)).min(len - 1);
    Some((start, end))
}

#[cfg(all(not(target_arch = "wasm32"), feature = "desktop"))]
mod imp {
    use std::sync::{Arc, OnceLock, RwLock};

    use axum::Router;
    use axum::extract::{Path, State};
    use axum::http::{HeaderMap, header};
    use axum::response::Response;
    use axum::routing::get;
    use dioxus::prelude::*;
    use tokio::io::{AsyncReadExt, AsyncSeekExt};

    use halogen_api::ApiClient;
    use halogen_ui_config::ClientConfig;
    use halogen_ui_logging::{debug, error, info, warn};
    use halogen_ui_svc_player::webview::{LOCAL_AUDIO_HANDLER, MEDIA_PROXY_HANDLER};

    use super::{
        STREAM_CHUNK_BYTES, allowed_media_path, clamp_range, mime_for_name, parse_range,
        safe_audio_file_name,
    };

    /// Live (server_url, token) snapshot for the server task — mirrored from
    /// the config signal (which is `!Send` and main-thread only) by the
    /// provider below. Process-global: the server outlives provider remounts.
    static AUTH: RwLock<Option<(String, Option<String>)>> = RwLock::new(None);

    /// Reqwest client reuse (connection pool) per server URL; token refreshed
    /// from [`AUTH`] on every request.
    static CLIENT: RwLock<Option<(String, Arc<ApiClient>)>> = RwLock::new(None);

    /// One server per process.
    static STARTED: OnceLock<()> = OnceLock::new();

    pub(super) fn install() {
        let config = use_context::<Signal<ClientConfig>>();

        use_hook(move || {
            // Seed the auth snapshot BEFORE the first paint so the very first
            // artwork requests don't race the mirror effect below.
            set_auth(&config.peek());
            start_media_server();
        });

        // Keep the snapshot current (login/logout/switch/token refresh).
        use_effect(move || {
            set_auth(&config.read());
        });
    }

    fn set_auth(cfg: &ClientConfig) {
        *AUTH.write().unwrap() = cfg
            .server_url
            .clone()
            .map(|url| (url, cfg.access_token.clone()));
    }

    /// Bind the loopback listener (synchronously — the port must be known
    /// before the first URL is built), publish the base, and serve on the
    /// ambient tokio runtime. Failures log and leave the base unset: media
    /// then falls back to relative URLs, i.e. broken art/audio but a live app.
    fn start_media_server() {
        STARTED.get_or_init(|| {
            let listener = match std::net::TcpListener::bind(("127.0.0.1", 0)) {
                Ok(l) => l,
                Err(e) => {
                    error!("media bridge: failed to bind loopback listener: {e}");
                    return;
                }
            };
            let port = match listener.local_addr() {
                Ok(a) => a.port(),
                Err(e) => {
                    error!("media bridge: no local addr: {e}");
                    return;
                }
            };
            let Ok(handle) = tokio::runtime::Handle::try_current() else {
                error!("media bridge: no tokio runtime; native media disabled");
                return;
            };

            // Per-process URL secret: the server is loopback-reachable by any
            // local process, so requests must carry this unguessable prefix.
            let nonce = format!("{:032x}", rand::random::<u128>());
            let state = ServerState {
                nonce: Arc::from(nonce.as_str()),
            };
            let app = Router::new()
                .route(
                    &format!("/{{nonce}}/{LOCAL_AUDIO_HANDLER}/{{file}}"),
                    get(serve_local_audio),
                )
                .route(
                    &format!("/{{nonce}}/{MEDIA_PROXY_HANDLER}/{{*path}}"),
                    get(proxy_server_media),
                )
                .with_state(state);

            handle.spawn(async move {
                if let Err(e) = listener.set_nonblocking(true) {
                    error!("media bridge: set_nonblocking failed: {e}");
                    return;
                }
                let listener = match tokio::net::TcpListener::from_std(listener) {
                    Ok(l) => l,
                    Err(e) => {
                        error!("media bridge: tokio listener: {e}");
                        return;
                    }
                };
                if let Err(e) = axum::serve(listener, app).await {
                    error!("media bridge: server exited: {e}");
                }
            });

            halogen_ui_appstate::media_url::set_local_media_base(format!(
                "http://127.0.0.1:{port}/{nonce}"
            ));
            info!("media bridge listening on 127.0.0.1:{port}");
        });
    }

    #[derive(Clone)]
    struct ServerState {
        nonce: Arc<str>,
    }

    /// CORS headers for every bridge response: on iOS/macOS the app page's
    /// origin is the custom scheme `dioxus://index.html`, which WKWebView
    /// treats as opaque and CORS-checks even plain `<img>` loads — and it makes
    /// them *credentialed*, so a `*` wildcard is rejected too ("Cannot use
    /// wildcard … when credentials flag is true"). The exact request origin
    /// must be echoed back with allow-credentials. Safe here: the routes are
    /// nonce-guarded, read-only media; the origin is whatever webview page the
    /// request came from, which on this loopback server is always our own app.
    fn with_cors(
        mut response: Response<axum::body::Body>,
        origin: Option<axum::http::HeaderValue>,
    ) -> Response<axum::body::Body> {
        let h = response.headers_mut();
        match origin {
            Some(o) => {
                h.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, o);
                h.insert(
                    header::ACCESS_CONTROL_ALLOW_CREDENTIALS,
                    axum::http::HeaderValue::from_static("true"),
                );
                h.insert(header::VARY, axum::http::HeaderValue::from_static("Origin"));
            }
            // No Origin header (native media stacks, curl) → nothing enforces
            // CORS; the permissive wildcard keeps non-credentialed loads happy.
            None => {
                h.insert(
                    header::ACCESS_CONTROL_ALLOW_ORIGIN,
                    axum::http::HeaderValue::from_static("*"),
                );
            }
        }
        response
    }

    fn plain_status(status: u16) -> Response<axum::body::Body> {
        Response::builder()
            .status(status)
            .body(axum::body::Body::empty())
            .expect("static status response")
    }

    fn bytes_response(
        status: u16,
        content_type: &str,
        extra: &[(header::HeaderName, String)],
        bytes: Vec<u8>,
    ) -> Response<axum::body::Body> {
        let mut builder = Response::builder()
            .status(status)
            .header(header::CONTENT_TYPE, content_type)
            .header(header::ACCEPT_RANGES, "bytes")
            .header(header::CONTENT_LENGTH, bytes.len().to_string());
        for (name, value) in extra {
            builder = builder.header(name, value);
        }
        builder
            .body(axum::body::Body::from(bytes))
            .unwrap_or_else(|_| plain_status(500))
    }

    /// `/{nonce}/halogen-local-audio/{file}` — device audio with Range support.
    async fn serve_local_audio(
        Path((nonce, file)): Path<(String, String)>,
        headers: HeaderMap,
        State(state): State<ServerState>,
    ) -> Response<axum::body::Body> {
        let origin = headers.get(header::ORIGIN).cloned();
        with_cors(
            serve_local_audio_inner(nonce, file, headers, state).await,
            origin,
        )
    }

    async fn serve_local_audio_inner(
        nonce: String,
        file: String,
        headers: HeaderMap,
        state: ServerState,
    ) -> Response<axum::body::Body> {
        if nonce != *state.nonce || !safe_audio_file_name(&file) {
            return plain_status(404);
        }
        let path = halogen_ui_svc_media::audio_dir().join(&file);
        let Ok(mut fh) = tokio::fs::File::open(&path).await else {
            return plain_status(404);
        };
        let Ok(len) = fh.metadata().await.map(|m| m.len()) else {
            return plain_status(404);
        };
        let mime = mime_for_name(&file);
        let range = headers
            .get(header::RANGE)
            .and_then(|v| v.to_str().ok())
            .and_then(parse_range);

        match range {
            Some((start, end)) => {
                let Some((start, end)) = clamp_range(start, end, len) else {
                    return Response::builder()
                        .status(416)
                        .header(header::CONTENT_RANGE, format!("bytes */{len}"))
                        .body(axum::body::Body::empty())
                        .unwrap_or_else(|_| plain_status(416));
                };
                let count = (end - start + 1) as usize;
                let mut bytes = vec![0u8; count];
                if fh.seek(std::io::SeekFrom::Start(start)).await.is_err()
                    || fh.read_exact(&mut bytes).await.is_err()
                {
                    return plain_status(500);
                }
                bytes_response(
                    206,
                    mime,
                    &[(header::CONTENT_RANGE, format!("bytes {start}-{end}/{len}"))],
                    bytes,
                )
            }
            None => {
                // No (or unsupported) Range: the whole file — STREAMED, never
                // buffered (a 100+ MB episode in one Vec was a real OOM risk on
                // mobile). Rare — the webview media stacks issue ranged
                // requests for audio.
                let stream = futures::stream::unfold((fh, false), |(mut fh, done)| async move {
                    if done {
                        return None;
                    }
                    let mut buf = vec![0u8; 512 * 1024];
                    match tokio::io::AsyncReadExt::read(&mut fh, &mut buf).await {
                        Ok(0) => None,
                        Ok(n) => {
                            buf.truncate(n);
                            Some((Ok(axum::body::Bytes::from(buf)), (fh, false)))
                        }
                        Err(e) => Some((Err(e), (fh, true))),
                    }
                });
                Response::builder()
                    .status(200)
                    .header(header::CONTENT_TYPE, mime)
                    .header(header::CONTENT_LENGTH, len)
                    .header(header::ACCEPT_RANGES, "bytes")
                    .body(axum::body::Body::from_stream(stream))
                    .unwrap_or_else(|_| plain_status(500))
            }
        }
    }

    /// `/{nonce}/halogen-media/{path}` — authenticated proxy to the server's
    /// media endpoints, Range forwarded (bounded).
    async fn proxy_server_media(
        Path((nonce, rest)): Path<(String, String)>,
        headers: HeaderMap,
        State(state): State<ServerState>,
    ) -> Response<axum::body::Body> {
        let origin = headers.get(header::ORIGIN).cloned();
        with_cors(
            proxy_server_media_inner(nonce, rest, headers, state).await,
            origin,
        )
    }

    async fn proxy_server_media_inner(
        nonce: String,
        rest: String,
        headers: HeaderMap,
        state: ServerState,
    ) -> Response<axum::body::Body> {
        if nonce != *state.nonce {
            return plain_status(404);
        }
        if !allowed_media_path(&rest) {
            debug!("media bridge refused path: {rest}");
            return plain_status(404);
        }

        let is_audio = rest.ends_with("/audio");
        let range = if is_audio {
            match headers
                .get(header::RANGE)
                .and_then(|v| v.to_str().ok())
                .and_then(parse_range)
            {
                // Explicit ends are capped like open ones: the upstream body is
                // buffered whole (`fetch_media_raw`), so an uncapped explicit
                // range holds the entire file in RAM. Fewer-bytes-than-asked
                // with a correct Content-Range is valid; players follow up.
                Some((start, end)) => {
                    let cap = start + STREAM_CHUNK_BYTES - 1;
                    Some(format!("bytes={start}-{}", end.map_or(cap, |e| e.min(cap))))
                }
                None => None,
            }
        } else {
            None
        };

        let Some(client) = resolve_client() else {
            return plain_status(401);
        };
        match client.fetch_media_raw(&rest, range.as_deref()).await {
            Ok(raw) => {
                let mut extra: Vec<(header::HeaderName, String)> = Vec::new();
                if let Some(cr) = raw.content_range {
                    extra.push((header::CONTENT_RANGE, cr));
                }
                // Let the webview's HTTP cache do its job for artwork.
                if let Some(cc) = raw.cache_control {
                    extra.push((header::CACHE_CONTROL, cc));
                }
                if let Some(etag) = raw.etag {
                    extra.push((header::ETAG, etag));
                }
                bytes_response(
                    raw.status,
                    raw.content_type
                        .as_deref()
                        .unwrap_or("application/octet-stream"),
                    &extra,
                    raw.bytes,
                )
            }
            Err(e) => {
                warn!("media bridge proxy fetch failed for {rest}: {e}");
                plain_status(502)
            }
        }
    }

    /// The API client for the current auth snapshot: pooled per server URL,
    /// token refreshed per request. Built via `api_client_from` (URL parse +
    /// scheme allowlist), deliberately NOT `ClientConfig::api_client`: manual
    /// offline must not black-hole media here, matching web where
    /// `<img>`/`<audio>` loads bypass that gate.
    fn resolve_client() -> Option<Arc<ApiClient>> {
        let (server_url, token) = AUTH.read().unwrap().clone()?;
        {
            let cached = CLIENT.read().unwrap();
            if let Some((url, client)) = cached.as_ref()
                && *url == server_url
            {
                client.set_token(token);
                return Some(client.clone());
            }
        }
        let built = Arc::new(halogen_ui_config::api_client_from(Some(&server_url), None)?);
        built.set_token(token);
        *CLIENT.write().unwrap() = Some((server_url, built.clone()));
        Some(built)
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    #[test]
    fn parse_range_forms() {
        assert_eq!(parse_range("bytes=0-"), Some((0, None)));
        assert_eq!(parse_range("bytes=100-200"), Some((100, Some(200))));
        assert_eq!(parse_range(" bytes=5-9 ".trim()), Some((5, Some(9))));
        assert_eq!(parse_range("bytes=-500"), None, "suffix ranges unsupported");
        assert_eq!(parse_range("bytes=9-5"), None, "inverted range");
        assert_eq!(parse_range("items=0-1"), None, "unknown unit");
        assert_eq!(parse_range("bytes=abc-"), None);
    }

    #[test]
    fn allowed_media_paths_only() {
        assert!(allowed_media_path("episodes/42/art"));
        assert!(allowed_media_path("episodes/42/art/small"));
        assert!(allowed_media_path("podcasts/7/art"));
        assert!(allowed_media_path("podcasts/7/art/small"));
        assert!(allowed_media_path("episodes/42/audio"));

        assert!(!allowed_media_path("episodes/42"));
        assert!(!allowed_media_path("episodes/42/audio/extra"));
        assert!(!allowed_media_path("users/1/art"));
        assert!(!allowed_media_path("episodes/../auth/refresh"));
        assert!(!allowed_media_path("episodes/x/audio"));
        assert!(!allowed_media_path(""));
    }

    #[test]
    fn safe_audio_file_names_only() {
        assert!(safe_audio_file_name("42.mp3"));
        assert!(safe_audio_file_name("42.partial"));
        assert!(!safe_audio_file_name(""));
        assert!(!safe_audio_file_name("../42.mp3"));
        assert!(!safe_audio_file_name("a/b.mp3"));
        assert!(!safe_audio_file_name("a\\b.mp3"));
        assert!(!safe_audio_file_name(".hidden"));
    }

    #[test]
    fn mime_mapping_mirrors_media_store_extensions() {
        assert_eq!(mime_for_name("1.mp3"), "audio/mpeg");
        assert_eq!(mime_for_name("1.m4a"), "audio/mp4");
        assert_eq!(mime_for_name("1.ogg"), "audio/ogg");
        assert_eq!(mime_for_name("1.opus"), "audio/opus");
        assert_eq!(mime_for_name("1.wav"), "audio/wav");
        assert_eq!(mime_for_name("1.flac"), "audio/flac");
        assert_eq!(mime_for_name("1.bin"), "application/octet-stream");
        assert_eq!(mime_for_name("noext"), "application/octet-stream");
    }

    #[test]
    fn clamp_range_caps_open_ended_and_respects_eof() {
        // Open-ended from 0 on a big file: capped to the chunk size.
        assert_eq!(
            clamp_range(0, None, 100 * 1024 * 1024),
            Some((0, STREAM_CHUNK_BYTES - 1))
        );
        // Open-ended near EOF: clamped to the last byte.
        assert_eq!(clamp_range(90, None, 100), Some((90, 99)));
        // Explicit end past EOF: clamped.
        assert_eq!(clamp_range(10, Some(1000), 100), Some((10, 99)));
        // Explicit end past the chunk cap: capped like an open-ended range —
        // the body is buffered whole, so `bytes=0-<huge>` must not hold the
        // entire file in RAM (the player follows up from the served end).
        assert_eq!(
            clamp_range(0, Some(u64::MAX - 1), 100 * 1024 * 1024),
            Some((0, STREAM_CHUNK_BYTES - 1))
        );
        // Start past EOF: unsatisfiable.
        assert_eq!(clamp_range(100, None, 100), None);
        assert_eq!(clamp_range(500, Some(600), 100), None);
    }
}
