//! Optionally serve public_root at public_url_path for development frontend hosting. ServeDir handles index files and
//! falls back to index.html for SPA deep links; production normally embeds assets.

use std::path::Path;

use axum::Router;
use axum::http::{HeaderValue, header};
use tower::ServiceBuilder;
use tower_http::compression::CompressionLayer;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::set_header::SetResponseHeaderLayer;

/// Serve root files with index.html fallback and frontend CSP. At `/`, use the router fallback so API routes win;
/// otherwise mount under url_path.
pub fn mount(router: Router, url_path: &str, root: &Path) -> Router {
    let index = root.join("index.html");
    let csp = SetResponseHeaderLayer::if_not_present(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(halogen_router::FRONTEND_CSP),
    );
    // Dev assets carry stable (un-hashed) names, so never let a browser pin a
    // stale build: `no-cache` means "cache but always revalidate" — `ServeDir`
    // answers a matching `If-Modified-Since` with a cheap 304, not a re-download.
    // (Production serves the embedded, content-hashed bundle with `immutable`.)
    let cache = SetResponseHeaderLayer::if_not_present(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-cache"),
    );
    // Serve the prebuilt `.br`/`.gz` siblings dx emits (e.g. the 5 MB wasm ships as a ~1 MB `.br`) when the
    // client accepts them, then compress on the fly anything left as identity (e.g. tailwind.css).
    // `CompressionLayer` skips responses that already carry a `Content-Encoding`, so the two compose without
    // double work.
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
