//! Optional "public directory" server.
//!
//! When `config.enable_public_server` is set and `config.public_root` points at a
//! directory, that directory is served at `config.public_url_path` (default `/`).
//! This is mainly a dev convenience for hosting the built frontend: pointed at a
//! directory with an `index.html`, `/` serves the app and unknown paths fall back
//! to `index.html` so client-side routing works. Production assets are normally
//! bundled into the binary instead.
//!
//! `ServeDir` already serves `index.html` for directory requests
//! (`append_index_html_on_directories` is on by default); we additionally set a
//! `not_found_service` to `index.html` so SPA deep links (e.g. `/queue`) resolve.

use std::path::Path;

use axum::Router;
use axum::http::{HeaderValue, header};
use tower::ServiceBuilder;
use tower_http::compression::CompressionLayer;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::set_header::SetResponseHeaderLayer;

/// Mount the public directory onto `router` at `url_path`.
///
/// Serves files from `root`, falling back to `index.html` (so a directory-mounted
/// SPA frontend works for client-side routes). When `url_path` is `/`, it is
/// installed as the **fallback** service (the last resort), so all real routes
/// take precedence and anything unmatched serves the frontend. Otherwise it is
/// nested at the given path (e.g. `/static`).
///
/// Every response carries the frontend CSP (see [`super::FRONTEND_CSP`]) — the
/// client must never load resources from origins outside this server.
pub fn mount(router: Router, url_path: &str, root: &Path) -> Router {
    let index = root.join("index.html");
    let csp = SetResponseHeaderLayer::if_not_present(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(super::FRONTEND_CSP),
    );
    // Dev assets carry stable (un-hashed) names, so never let a browser pin a
    // stale build: `no-cache` means "cache but always revalidate" — `ServeDir`
    // answers a matching `If-Modified-Since` with a cheap 304, not a re-download.
    // (Production serves the embedded, content-hashed bundle with `immutable`.)
    let cache = SetResponseHeaderLayer::if_not_present(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-cache"),
    );
    // Serve the prebuilt `.br`/`.gz` siblings dx emits (e.g. the 5 MB wasm ships
    // as a ~1 MB `.br`) when the client accepts them, then compress on the fly
    // anything left as identity (e.g. tailwind.css). `CompressionLayer` skips
    // responses that already carry a `Content-Encoding`, so the two compose
    // without double work.
    let serve = ServeDir::new(root)
        .precompressed_br()
        .precompressed_gzip()
        .not_found_service(ServeFile::new(index));
    let svc = ServiceBuilder::new()
        .layer(csp)
        .layer(cache)
        .layer(CompressionLayer::new())
        .service(serve);
    if url_path == "/" {
        router.fallback_service(svc)
    } else {
        router.nest_service(url_path, svc)
    }
}
