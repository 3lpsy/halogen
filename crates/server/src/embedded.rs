//! Serve the `ui-build` frontend embedded in the binary, with SPA route fallback.
//! Stable build filenames revalidate with ETags; optional precompressed assets
//! are served directly and remaining responses use runtime compression.

use std::convert::Infallible;
use std::fmt::Write as _;

use axum::Router;
use axum::body::Body;
use axum::extract::Request;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use rust_embed::{EmbeddedFile, RustEmbed};
use tower::{ServiceBuilder, service_fn};
use tower_http::compression::CompressionLayer;

// Include the content digest in sccache's tracked environment dependencies.
const _: &str = env!("HALOGEN_FRONTEND_DIGEST");
include!(concat!(env!("OUT_DIR"), "/frontend_dependencies.rs"));

// `folder` is resolved relative to CARGO_MANIFEST_DIR (crates/server), so this
// points at the workspace-root dist/. (rust-embed only expands `$VARS` with the
// `interpolate-folder-path` feature, which we don't need.)
#[derive(RustEmbed)]
#[folder = "../../dist"]
struct Assets;

/// Install the embedded frontend as the router fallback. The fallback is wrapped in a [`CompressionLayer`] so
/// identity responses (index.html, tailwind.css) are compressed on the fly; precompressed assets served by
/// [`serve`] already carry a `Content-Encoding` and are skipped.
pub fn mount(router: Router) -> Router {
    let svc = ServiceBuilder::new()
        .layer(CompressionLayer::new())
        .service(service_fn(|req: Request| async move {
            Ok::<_, Infallible>(serve(req).await)
        }));
    router.fallback_service(svc)
}

/// Whether `Accept-Encoding` offers `enc`. Ignores q-values (a `q=0` rejection
/// is rare enough that the only cost of getting it wrong is one wasted encode).
fn accepts(req: &Request, enc: &str) -> bool {
    req.headers()
        .get(header::ACCEPT_ENCODING)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|val| {
            val.split(',').any(|tok| {
                tok.split(';')
                    .next()
                    .map(str::trim)
                    .is_some_and(|name| name.eq_ignore_ascii_case(enc))
            })
        })
}

/// Strong ETag (quoted hex sha256) of an embedded file. rust-embed computes the
/// hash at compile time, so this is just formatting at runtime.
fn etag_of(asset: &EmbeddedFile) -> String {
    let hash = asset.metadata.sha256_hash();
    let mut tag = String::with_capacity(hash.len() * 2 + 2);
    tag.push('"');
    for byte in hash {
        let _ = write!(tag, "{byte:02x}");
    }
    tag.push('"');
    tag
}

/// Stable PWA root icons: un-hashed names whose bytes effectively never change (a rebrand is rare and a few
/// days of staleness is harmless). They were being served `no-cache`, so a 132 KB favicon re-revalidated
/// (conditional GET) on every client-side route change. A real `max-age` lets the browser reuse them without a
/// round trip while still picking up a redeploy within a week.
fn is_long_cache_icon(path: &str) -> bool {
    let path = path.strip_prefix("icons/").unwrap_or(path);
    matches!(
        path,
        "favicon.ico" | "icon.svg" | "apple-touch-icon.png" | "icon-192.png" | "icon-512.png"
    ) || (path.starts_with("icon-") && path.ends_with(".png"))
}

/// Fixed-name app files revalidate on deployment; icons get a week.
/// Legacy content-hashed assets remain immutable.
fn cache_control(path: &str) -> &'static str {
    if path.contains("-dxh") {
        "public, max-age=31536000, immutable"
    } else if is_long_cache_icon(path) {
        "public, max-age=604800"
    } else {
        "no-cache"
    }
}

/// Serve an embedded asset by request path, falling back to `index.html` so
/// client-side routes (e.g. `/queue`) resolve.
async fn serve(req: Request) -> Response {
    let raw = req.uri().path().trim_start_matches('/');
    let req_path = if raw.is_empty() { "index.html" } else { raw };

    // Resolve the asset; unknown paths fall back to the SPA shell.
    let (path, asset) = match Assets::get(req_path) {
        Some(asset) => (req_path.to_string(), asset),
        None => match Assets::get("index.html") {
            Some(asset) => ("index.html".to_string(), asset),
            None => {
                return (
                    StatusCode::NOT_FOUND,
                    "embedded frontend not found (was dist/ built before compiling with \
                     --features embed-frontend?)",
                )
                    .into_response();
            }
        },
    };

    // Content-Type always reflects the ORIGINAL asset — a `.br` sibling would
    // mime-guess to octet-stream.
    let mime = asset.metadata.mimetype().to_string();

    // Prefer a prebuilt precompressed sibling the client accepts; the ETag comes
    // from the bytes actually served, so each encoding has a distinct validator.
    let (encoding, served) = if accepts(&req, "br") {
        match Assets::get(&format!("{path}.br")) {
            Some(br) => (Some("br"), br),
            None => (None, asset),
        }
    } else if accepts(&req, "gzip") {
        match Assets::get(&format!("{path}.gz")) {
            Some(gz) => (Some("gzip"), gz),
            None => (None, asset),
        }
    } else {
        (None, asset)
    };

    let etag = etag_of(&served);
    let cache = cache_control(&path);

    // Conditional GET: a matching validator means the client already has these
    // exact bytes — answer 304 and skip the body.
    let not_modified = req
        .headers()
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|inm| inm.split(',').any(|t| t.trim() == etag));
    if not_modified {
        return Response::builder()
            .status(StatusCode::NOT_MODIFIED)
            .header(header::CACHE_CONTROL, cache)
            .header(header::ETAG, &etag)
            .header(header::VARY, header::ACCEPT_ENCODING.as_str())
            .body(Body::empty())
            .expect("build 304 response");
    }

    let mut builder = Response::builder()
        .header(header::CONTENT_TYPE, mime)
        // Enforce the no-external-origins rule on every frontend asset.
        .header(
            header::CONTENT_SECURITY_POLICY,
            halogen_router::FRONTEND_CSP,
        )
        .header(header::CACHE_CONTROL, cache)
        .header(header::ETAG, &etag)
        .header(header::VARY, header::ACCEPT_ENCODING.as_str());
    if let Some(encoding) = encoding {
        builder = builder.header(header::CONTENT_ENCODING, encoding);
    }
    builder
        .body(Body::from(served.data.into_owned()))
        .expect("build embedded asset response")
}

#[cfg(test)]
mod tests {
    use super::cache_control;

    #[test]
    fn content_hashed_assets_are_immutable() {
        assert_eq!(
            cache_control("assets/halogen-webui-dxh3238b1d64ea12aa.js"),
            "public, max-age=31536000, immutable"
        );
    }

    #[test]
    fn stable_pwa_icons_get_a_real_max_age() {
        // These were served `no-cache`, forcing a conditional GET (a 132 KB favicon)
        // on every client-side route change — Lighthouse saw ~15 refetches.
        for p in [
            "favicon.ico",
            "icon.svg",
            "apple-touch-icon.png",
            "icon-192.png",
            "icon-512.png",
            "icon-maskable-512.png",
            "icons/icon-192.png",
            "icons/icon-maskable-512.png",
        ] {
            assert_eq!(cache_control(p), "public, max-age=604800", "{p}");
        }
    }

    #[test]
    fn shell_and_manifest_still_revalidate() {
        // Fixed-name shell files must revalidate so a redeploy is seen.
        for p in [
            "index.html",
            "manifest.webmanifest",
            "tailwind.css",
            "webui.js",
            "webui_bg.wasm",
        ] {
            assert_eq!(cache_control(p), "no-cache", "{p}");
        }
    }
}
