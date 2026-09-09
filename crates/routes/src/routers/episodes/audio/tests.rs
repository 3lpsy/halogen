use crate::routers::episodes::tests::{
    build_test_router, generate_jwt_token, generate_media_jwt_token, setup_test_db,
};
use crate::tests::harness::{authed, json_body};
use axum::body::Body;
use axum::http::{Request, StatusCode};
use halogen_orm::episode::ActiveModel as EpisodeActiveModel;
use halogen_wire::DownloadStatus;
use sea_orm::ActiveModelTrait;
use tower::ServiceExt;

/// Build a `GET /episodes/{id}/audio` request, optionally carrying an
/// `auth_media` cookie value.
fn audio_req(id: &str, cookie: Option<&str>) -> Request<Body> {
    let mut b = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/episodes/{id}/audio"));
    if let Some(c) = cookie {
        b = b.header("Cookie", format!("auth_media={c}"));
    }
    b.body(Body::empty()).unwrap()
}

// No `auth_media` cookie at all → 401 (the handler reads the cookie itself).
#[tokio::test]
async fn test_audio_missing_cookie_is_unauthorized() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);
    let episode_id = payload.get("episode_id_1").unwrap().as_str().unwrap();

    let response = router
        .clone()
        .oneshot(audio_req(episode_id, None))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

// A garbage cookie value fails JWT decode → 401.
#[tokio::test]
async fn test_audio_invalid_cookie_is_unauthorized() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);
    let episode_id = payload.get("episode_id_1").unwrap().as_str().unwrap();

    let response = router
        .clone()
        .oneshot(audio_req(episode_id, Some("not-a-valid-jwt")))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

/// Build a `GET /episodes/{id}/audio` request carrying a bearer token.
fn audio_req_bearer(id: &str, token: &str) -> Request<Body> {
    authed("GET", format!("/api/v1/episodes/{id}/audio"), token)
}

// A normal API token as bearer authenticates (programmatic byte fetches —
// the device-download flow). The seeded episode isn't downloaded, so a 404
// (not 401) proves auth passed.
#[tokio::test]
async fn test_audio_api_bearer_is_authorized() {
    let (_root, dbc, payload) = setup_test_db().await;
    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let api_token = generate_jwt_token(user_id);
    let episode_id = payload.get("episode_id_1").unwrap().as_str().unwrap();
    let router = build_test_router(dbc);

    let response = router
        .clone()
        .oneshot(audio_req_bearer(episode_id, &api_token))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

// A media-scoped token as bearer is rejected — a leaked streaming cookie
// value must not work as an API credential (scope/transport lock).
#[tokio::test]
async fn test_audio_media_token_as_bearer_is_unauthorized() {
    let (_root, dbc, payload) = setup_test_db().await;
    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let media = generate_media_jwt_token(user_id);
    let episode_id = payload.get("episode_id_1").unwrap().as_str().unwrap();
    let router = build_test_router(dbc);

    let response = router
        .clone()
        .oneshot(audio_req_bearer(episode_id, &media))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

// A valid *API* token in the cookie is rejected — only media-scoped tokens
// stream (cross-use guard).
#[tokio::test]
async fn test_audio_api_token_in_cookie_is_unauthorized() {
    let (_root, dbc, payload) = setup_test_db().await;
    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let api_token = generate_jwt_token(user_id);
    let episode_id = payload.get("episode_id_1").unwrap().as_str().unwrap();
    let router = build_test_router(dbc);

    let response = router
        .clone()
        .oneshot(audio_req(episode_id, Some(&api_token)))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

// Valid media cookie, but the seeded episode is `NotDownloaded` (no local
// file) → the server only serves downloaded copies, so it 404s.
#[tokio::test]
async fn test_audio_not_downloaded_is_not_found() {
    let (_root, dbc, payload) = setup_test_db().await;
    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let media = generate_media_jwt_token(user_id);
    let episode_id = payload.get("episode_id_1").unwrap().as_str().unwrap();
    let router = build_test_router(dbc);

    let response = router
        .clone()
        .oneshot(audio_req(episode_id, Some(&media)))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

// A valid media cookie for a user subscribed to nothing must NOT stream the
// episode. The media path does no DB existence check, so the token alone
// authenticates — the subscription guard still 404s. The validation-error
// body (code "exists") distinguishes it from the plain-text "not downloaded".
#[tokio::test]
async fn test_audio_hidden_from_non_subscriber() {
    let (_root, dbc, payload) = setup_test_db().await;
    let episode_id = payload.get("episode_id_1").unwrap().as_str().unwrap();
    let outsider = generate_media_jwt_token(&(i32::MAX - 500).to_string());
    let router = build_test_router(dbc);

    let response = router
        .oneshot(audio_req(episode_id, Some(&outsider)))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let json = json_body(response).await;
    let code = json
        .get("errors")
        .and_then(|e| e.get("id"))
        .and_then(|i| i.as_array())
        .and_then(|a| a.first())
        .and_then(|f| f.get("code"))
        .and_then(|c| c.as_str())
        .expect("subscription guard error code");
    assert_eq!(code, "exists", "non-subscriber must not stream the episode");
}

// A non-integer id is rejected by the `Path<i32>` extractor with a 400, even
// with a valid media cookie.
#[tokio::test]
async fn test_audio_invalid_id_is_bad_request() {
    let (_root, dbc, payload) = setup_test_db().await;
    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let media = generate_media_jwt_token(user_id);
    let router = build_test_router(dbc);

    let response = router
        .clone()
        .oneshot(audio_req("abc", Some(&media)))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

// Valid media cookie, but the episode is mid-download (`Downloading`) with no
// file → still 404 (only `Downloaded` + a path streams).
#[tokio::test]
async fn test_audio_downloading_status_is_not_found() {
    let (_root, dbc, payload) = setup_test_db().await;
    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let media = generate_media_jwt_token(user_id);
    let episode_id: i32 = payload
        .get("episode_id_2")
        .unwrap()
        .as_str()
        .unwrap()
        .parse()
        .unwrap();

    // Flip the seeded episode into a `Downloading` state before serving.
    let update = EpisodeActiveModel {
        id: sea_orm::ActiveValue::set(episode_id),
        download_status: sea_orm::ActiveValue::set(DownloadStatus::Downloading),
        ..Default::default()
    };
    update.update(&dbc).await.expect("update episode status");

    let router = build_test_router(dbc);

    let response = router
        .clone()
        .oneshot(audio_req(&episode_id.to_string(), Some(&media)))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

// A downloaded episode whose stored path is OUTSIDE media_root must NOT be
// served — defense-in-depth against a stray/hostile absolute path (the
// arbitrary-file-read class). The file exists (so `ServeFile` alone would
// serve it); the media_root confinement is what turns it into a 404.
#[tokio::test]
async fn test_audio_path_outside_media_root_is_refused() {
    use sea_orm::ActiveValue;

    let (root, dbc, payload) = setup_test_db().await;
    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let media = generate_media_jwt_token(user_id);
    let episode_id: i32 = payload
        .get("episode_id_1")
        .unwrap()
        .as_str()
        .unwrap()
        .parse()
        .unwrap();

    // A real, readable file that lives outside the server's media_root.
    let outside = root.path().join("outside-media-root.mp3");
    std::fs::write(&outside, b"SECRET-BYTES").unwrap();

    let update = EpisodeActiveModel {
        id: ActiveValue::set(episode_id),
        download_status: ActiveValue::set(DownloadStatus::Downloaded),
        content_file_path: ActiveValue::set(Some(outside.display().to_string())),
        ..Default::default()
    };
    update
        .update(&dbc)
        .await
        .expect("mark downloaded (outside root)");

    let router = build_test_router(dbc);
    let response = router
        .oneshot(audio_req(&episode_id.to_string(), Some(&media)))
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::NOT_FOUND,
        "a content_file_path outside media_root must never be served"
    );
}
