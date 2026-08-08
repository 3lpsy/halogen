//! Auth journeys. `auth_journey` is one ordered flow whose steps build on each
//! other: public health probe → reject anonymous → reject bad creds → log in →
//! use the issued token → refresh → logout, driven through the real
//! `halogen_api::ApiClient` (the exact gateway the UI uses). Two raw-HTTP tests
//! cover what the typed client can't see: the `auth_media` cookie lifecycle +
//! login-issued-cookie streaming, and refresh rejecting a media-scoped token.
//!
//! Run with: `cargo nextest run -p halogen-integ -E 'binary(auth_flow)'`

use halogen_api::{LoginData, TokenData};
use halogen_integ::*;

#[tokio::test]
async fn auth_journey() {
    let app = spawn().await;
    let anon = anon_api(&app);

    // 1) The public health probe works without auth and decodes to StatusData,
    //    reporting the server is up.
    let health = anon.health().await.expect("health probe");
    assert!(health.running, "healthz reports the server is up");

    // 2) A protected route with no token is rejected (401).
    let err = anon.list_episodes(ep_page(0, 10)).await.unwrap_err();
    assert_eq!(status_of(&err), 401, "anonymous => 401");

    // 3) Seed an admin; logging in with the wrong password is rejected (401).
    let admin = app.seed_admin().await;
    let err = anon
        .login(LoginData {
            username: admin.username.clone(),
            password: "definitely-wrong".into(),
        })
        .await
        .unwrap_err();
    assert_eq!(status_of(&err), 401, "bad creds => 401");

    // 4) Correct creds return a non-empty token.
    let token = anon
        .login(LoginData {
            username: admin.username.clone(),
            password: admin.password.clone(),
        })
        .await
        .expect("login succeeds")
        .token;
    assert!(!token.is_empty(), "token should be non-empty");

    // 5) The freshly-issued token unlocks the protected route and returns data
    //    (an empty list — nothing seeded yet — not just a 200).
    let client = api(&app, &token);
    let page = client
        .list_episodes(ep_page(0, 10))
        .await
        .expect("valid token authorizes");
    assert!(page.data.is_empty(), "no episodes seeded yet");

    // 6) refresh returns a usable token that still authorizes a protected call.
    let refreshed = anon
        .refresh(TokenData {
            token: token.clone(),
        })
        .await
        .expect("refresh succeeds")
        .token;
    assert!(!refreshed.is_empty(), "refreshed token is non-empty");
    api(&app, &refreshed)
        .list_episodes(ep_page(0, 10))
        .await
        .expect("refreshed token authorizes");

    // 7) logout succeeds.
    client.logout().await.expect("logout succeeds");
}

/// The `auth_media` cookie lifecycle over raw HTTP (the typed client discards
/// response headers): login SETS it, logout and refresh CLEAR/re-issue it. And
/// the full issuance→consumption loop: log in, capture the cookie, and stream
/// audio with it → 200/206 (the existing media_flow mints the JWT directly;
/// this proves the login-issued cookie actually authorizes media).
#[tokio::test]
async fn auth_media_cookie_lifecycle_and_streaming() {
    let app = spawn().await;
    let admin = app.seed_admin().await;
    let http = reqwest::Client::new();
    let login_url = format!("{}/api/v1/auth/login", app.base_url);

    // Login sets the auth_media cookie (HttpOnly, scoped to /api/v1, Max-Age > 0).
    let resp = http
        .post(&login_url)
        .json(&serde_json::json!({
            "data": {"username": admin.username, "password": admin.password}
        }))
        .send()
        .await
        .expect("login");
    assert_eq!(resp.status().as_u16(), 200, "login ok");
    let set_cookie = resp
        .headers()
        .get(reqwest::header::SET_COOKIE)
        .expect("login sets a cookie")
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        set_cookie.starts_with("auth_media="),
        "login sets the auth_media cookie"
    );
    assert!(set_cookie.contains("HttpOnly"), "cookie is HttpOnly");
    assert!(
        set_cookie.contains("Path=/api/v1"),
        "cookie scoped to the API prefix"
    );
    assert!(
        !set_cookie.contains("Max-Age=0"),
        "login cookie has a live Max-Age"
    );
    // The body carries the API token; capture both.
    let body: serde_json::Value = resp.json().await.expect("login json");
    let api_token = body["data"]["token"].as_str().expect("token").to_string();
    // Extract the cookie value for streaming.
    let media_cookie_value = set_cookie
        .strip_prefix("auth_media=")
        .and_then(|s| s.split(';').next())
        .expect("cookie value")
        .to_string();

    // Logout CLEARS the cookie (Max-Age=0).
    let resp = http
        .post(format!("{}/api/v1/auth/logout", app.base_url))
        .send()
        .await
        .expect("logout");
    let cleared = resp
        .headers()
        .get(reqwest::header::SET_COOKIE)
        .expect("logout sets a cookie")
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        cleared.starts_with("auth_media=;"),
        "logout clears the value"
    );
    assert!(cleared.contains("Max-Age=0"), "logout expires the cookie");

    // Refresh RE-ISSUES the cookie (present, live) and returns a different token.
    let resp = http
        .post(format!("{}/api/v1/auth/refresh", app.base_url))
        .json(&serde_json::json!({"data": {"token": api_token}}))
        .send()
        .await
        .expect("refresh");
    assert_eq!(resp.status().as_u16(), 200, "refresh ok");
    let reissued = resp
        .headers()
        .get(reqwest::header::SET_COOKIE)
        .expect("refresh re-sets a cookie")
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        reissued.starts_with("auth_media=") && !reissued.contains("Max-Age=0"),
        "refresh re-issues a live cookie"
    );
    let rbody: serde_json::Value = resp.json().await.expect("refresh json");
    let refreshed_token = rbody["data"]["token"].as_str().expect("token").to_string();
    // NOTE: refresh re-issues with the same `sub` + same-second `exp`, so the JWT
    // bytes can be identical to the original — we do NOT assert the token differs.
    // What matters is that the re-issued token still authorizes a protected call.
    api(&app, &refreshed_token)
        .list_episodes(ep_page(0, 10))
        .await
        .expect("refreshed token authorizes");

    // ── Issuance → consumption: stream audio with the login-issued cookie ──
    let client = api(&app, &api_token);
    let (_podcast_id, episode_id) = subscribe_and_poll(&app, &client, "sed_podcast.xml").await;

    // Stage the episode's audio inside media_root so audio has bytes to serve.
    // (The write API no longer accepts a client-supplied `content_file_path`.)
    let payload: &[u8] = b"HALOGEN-AUTH-LOOP-AUDIO-XYZ";
    app.stage_downloaded_audio(episode_id, payload).await;

    let audio_url = format!("{}/api/v1/episodes/{episode_id}/audio", app.base_url);

    // Full request with the login-issued media cookie → 200.
    let resp = http
        .get(&audio_url)
        .header(
            reqwest::header::COOKIE,
            format!("auth_media={media_cookie_value}"),
        )
        .send()
        .await
        .expect("audio GET");
    assert_eq!(
        resp.status().as_u16(),
        200,
        "login-issued cookie streams audio"
    );

    // Range request → 206 (proves range support through the cookie auth path).
    let resp = http
        .get(&audio_url)
        .header(
            reqwest::header::COOKIE,
            format!("auth_media={media_cookie_value}"),
        )
        .header(reqwest::header::RANGE, "bytes=0-3")
        .send()
        .await
        .expect("audio range GET");
    assert_eq!(
        resp.status().as_u16(),
        206,
        "range request is Partial Content"
    );
}

/// Refresh is API-token-only: a media-scoped token cannot be upgraded into a
/// full-access token (the cross-use guard in the refresh handler) → 401.
#[tokio::test]
async fn refresh_rejects_media_scoped_token() {
    let app = spawn().await;
    let admin = app.seed_admin().await;
    let http = reqwest::Client::new();

    // A media-scoped token (the value the auth_media cookie carries).
    let media = app.media_jwt(&admin.id.to_string());

    let resp = http
        .post(format!("{}/api/v1/auth/refresh", app.base_url))
        .json(&serde_json::json!({"data": {"token": media}}))
        .send()
        .await
        .expect("refresh media token");
    assert_eq!(
        resp.status().as_u16(),
        401,
        "a media-scoped token cannot be refreshed into an API token"
    );
}
