//! CORS layer for the `/healthz` + `/api/v1` edge.

use axum::http::{Method, header};
use tower_http::cors::{AllowOrigin, CorsLayer};

/// Build the CORS layer from the configured allow-list. Both branches set `allow_credentials(true)` so the
/// browser sends the `auth_media` cookie cross-origin (dev: UI and API on different ports) and accepts the
/// credentialed audio response. Credentials forbid a `*` origin, so the dev branch reflects the request origin
/// rather than being wildcard. A non-empty list restricts `Access-Control-Allow-Origin` to those exact origins.
pub(super) fn build_cors(allowed_origins: &[String]) -> CorsLayer {
    let methods = [
        Method::GET,
        Method::POST,
        Method::PUT,
        Method::DELETE,
        Method::OPTIONS,
    ];

    if allowed_origins.is_empty() {
        tracing::warn!(
            "CORS: no allowed origins configured — reflecting the request origin (intended for dev)"
        );
        return CorsLayer::new()
            .allow_origin(AllowOrigin::mirror_request())
            .allow_methods(methods)
            .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE])
            .allow_credentials(true);
    }

    let origins: Vec<axum::http::HeaderValue> = allowed_origins
        .iter()
        .filter_map(|o| match o.parse() {
            Ok(v) => Some(v),
            Err(_) => {
                tracing::warn!(origin = %o, "CORS: ignoring invalid allowed origin");
                None
            }
        })
        .collect();

    CorsLayer::new()
        .allow_origin(AllowOrigin::list(origins))
        .allow_methods(methods)
        .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE])
        .allow_credentials(true)
}

#[cfg(test)]
mod tests {
    use super::build_cors;
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Method, Request, header};
    use axum::routing::get;
    use tower::ServiceExt;

    /// Empty allow-list → permissive layer. We can't introspect `CorsLayer`, so
    /// drive a preflight through it and confirm it reflects an arbitrary origin
    /// (permissive echoes whatever origin asks).
    #[tokio::test]
    async fn test_build_cors_empty_is_permissive() {
        let cors = build_cors(&[]);
        let app = Router::new().route("/", get(|| async { "ok" })).layer(cors);

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::OPTIONS)
                    .uri("/")
                    .header(header::ORIGIN, "https://anything.example")
                    .header(header::ACCESS_CONTROL_REQUEST_METHOD, "GET")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Permissive CORS allows the preflight and echoes the origin (as `*` or
        // the requesting origin). Either way the allow-origin header is present.
        assert!(
            response
                .headers()
                .contains_key(header::ACCESS_CONTROL_ALLOW_ORIGIN),
            "permissive policy should set an allow-origin header"
        );
    }

    /// A configured allow-list restricts `Access-Control-Allow-Origin` to the
    /// listed origin when that origin sends the preflight.
    #[tokio::test]
    async fn test_build_cors_configured_origin_is_allowed() {
        let cors = build_cors(&["https://example.com".to_string()]);
        let app = Router::new().route("/", get(|| async { "ok" })).layer(cors);

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::OPTIONS)
                    .uri("/")
                    .header(header::ORIGIN, "https://example.com")
                    .header(header::ACCESS_CONTROL_REQUEST_METHOD, "GET")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let allow_origin = response
            .headers()
            .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .and_then(|v| v.to_str().ok());
        assert_eq!(allow_origin, Some("https://example.com"));
    }

    /// An origin not on the configured list does not get an allow-origin echo.
    #[tokio::test]
    async fn test_build_cors_unlisted_origin_is_not_echoed() {
        let cors = build_cors(&["https://example.com".to_string()]);
        let app = Router::new().route("/", get(|| async { "ok" })).layer(cors);

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::OPTIONS)
                    .uri("/")
                    .header(header::ORIGIN, "https://evil.example")
                    .header(header::ACCESS_CONTROL_REQUEST_METHOD, "GET")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let allow_origin = response
            .headers()
            .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .and_then(|v| v.to_str().ok());
        assert_ne!(allow_origin, Some("https://evil.example"));
    }

    /// An invalid origin string is `filter_map`-skipped (logged, not panicked),
    /// so the layer still constructs and serves requests.
    #[tokio::test]
    async fn test_build_cors_invalid_origin_is_skipped_without_panic() {
        // Mix of one parseable origin and one garbage string.
        let cors = build_cors(&["not a url".to_string(), "https://valid.example".to_string()]);
        let app = Router::new().route("/", get(|| async { "ok" })).layer(cors);

        // The valid origin still works despite the garbage entry being dropped.
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::OPTIONS)
                    .uri("/")
                    .header(header::ORIGIN, "https://valid.example")
                    .header(header::ACCESS_CONTROL_REQUEST_METHOD, "GET")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert!(response.status().is_success());
        let allow_origin = response
            .headers()
            .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .and_then(|v| v.to_str().ok());
        assert_eq!(allow_origin, Some("https://valid.example"));
    }
}
