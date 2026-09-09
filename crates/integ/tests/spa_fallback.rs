//! Verify raw HTTP SPA paths such as `/podcasts` reach the frontend without API authentication; guarded resources
//! belong under `/api/v1`. Run the halogen-integ spa_fallback binary.

use halogen_integ::*;
use reqwest::StatusCode;

/// The guarded API still lives — under the prefix — and rejects an unauthenticated
/// request with the documented 401 body.
#[tokio::test]
async fn prefixed_api_is_guarded() {
    let app = spawn().await;
    let http = reqwest::Client::new();

    for path in ["/api/v1/podcasts", "/api/v1/playlists"] {
        let resp = http.get(app.url(path)).send().await.expect("request");
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "{path} must require auth"
        );
        let body = resp.text().await.unwrap();
        assert!(
            body.contains("Missing or malformed Authorization header"),
            "{path} returns the auth error, got: {body}"
        );
    }
}

/// The unprefixed SPA paths no longer hit a guarded API route. They must NOT
/// return the 401 auth error — in this harness (no SPA fallback mounted) they
/// 404, which is enough to prove the collision is gone.
#[tokio::test]
async fn spa_paths_do_not_hit_the_api() {
    let app = spawn().await;
    let http = reqwest::Client::new();

    for path in ["/podcasts", "/playlists", "/podcasts/1", "/episodes/1"] {
        let resp = http.get(app.url(path)).send().await.expect("request");
        assert_ne!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "{path} must not be guarded by the API auth middleware"
        );
        let body = resp.text().await.unwrap();
        assert!(
            !body.contains("Missing or malformed Authorization header"),
            "{path} must not return the API auth error, got: {body}"
        );
    }
}
