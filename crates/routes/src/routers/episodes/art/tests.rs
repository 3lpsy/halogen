use crate::routers::episodes::tests::{
    build_test_router, build_test_router_with_media_root, generate_jwt_token, setup_test_db,
};
use crate::tests::harness::{authed, json_body, unauthed};
use axum::http::StatusCode;
use tower::ServiceExt;

// No credential → 401 (same media auth as the audio endpoint).
#[tokio::test]
async fn test_art_unauthenticated_is_unauthorized() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);
    let episode_id = payload.get("episode_id_1").unwrap().as_str().unwrap();

    let response = router
        .oneshot(unauthed(
            "GET",
            format!("/api/v1/episodes/{episode_id}/art"),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

// Success path: a disk-cached art file is served with the day-long
// Cache-Control header (the browser-cache contract). The file is pre-placed
// and `art_file_path` set so `ensure_episode_art` short-circuits — no
// network egress.
#[tokio::test]
async fn test_art_success_sets_cache_control() {
    use sea_orm::{ActiveModelTrait, ActiveValue};

    let (root, dbc, payload) = setup_test_db().await;
    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(user_id);
    let episode_id: i32 = payload
        .get("episode_id_1")
        .unwrap()
        .as_str()
        .unwrap()
        .parse()
        .unwrap();

    let art_path = root.path().join("episode_art.png");
    std::fs::write(&art_path, b"\x89PNG\r\n\x1a\nfake").unwrap();
    let update = halogen_orm::episode::ActiveModel {
        id: ActiveValue::set(episode_id),
        art_file_path: ActiveValue::set(Some(art_path.display().to_string())),
        ..Default::default()
    };
    update.update(&dbc).await.unwrap();

    // The router serves only paths under media_root; stage into the root dir.
    let router = build_test_router_with_media_root(dbc, root.path().to_path_buf());
    let response = router
        .oneshot(authed(
            "GET",
            format!("/api/v1/episodes/{episode_id}/art"),
            &token,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(axum::http::header::CACHE_CONTROL)
            .expect("art response carries Cache-Control"),
        "private, max-age=86400"
    );
}

// /art/small route is wired and serves: a pre-placed (undecodable) original
// means thumbnail generation falls back to the original, so the endpoint
// still returns 200 with the day-long Cache-Control. Proves the new route +
// export + shared serve/auth path without network egress or a real decode.
#[tokio::test]
async fn test_art_small_serves_with_cache_control() {
    use sea_orm::{ActiveModelTrait, ActiveValue};

    let (root, dbc, payload) = setup_test_db().await;
    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(user_id);
    let episode_id: i32 = payload
        .get("episode_id_1")
        .unwrap()
        .as_str()
        .unwrap()
        .parse()
        .unwrap();

    let art_path = root.path().join("episode_art.png");
    std::fs::write(&art_path, b"\x89PNG\r\n\x1a\nfake").unwrap();
    let update = halogen_orm::episode::ActiveModel {
        id: ActiveValue::set(episode_id),
        art_file_path: ActiveValue::set(Some(art_path.display().to_string())),
        ..Default::default()
    };
    update.update(&dbc).await.unwrap();

    // The router serves only paths under media_root; stage into the root dir.
    let router = build_test_router_with_media_root(dbc, root.path().to_path_buf());
    let response = router
        .oneshot(authed(
            "GET",
            format!("/api/v1/episodes/{episode_id}/art/small"),
            &token,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(axum::http::header::CACHE_CONTROL)
            .expect("small art response carries Cache-Control"),
        "private, max-age=86400"
    );
}

// Bearer authenticates; the seeded episode has no `art_url`, so the cache
// has nothing to fetch → 204 (proves route + auth + service wiring without
// any network egress; 204 keeps the empty-art case out of the browser console).
#[tokio::test]
async fn test_art_bearer_no_artwork_is_no_content() {
    let (_root, dbc, payload) = setup_test_db().await;
    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(user_id);
    let episode_id = payload.get("episode_id_1").unwrap().as_str().unwrap();
    let router = build_test_router(dbc);

    let response = router
        .oneshot(authed(
            "GET",
            format!("/api/v1/episodes/{episode_id}/art"),
            &token,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NO_CONTENT);
}

// A bearer for a user subscribed to nothing must NOT fetch/serve the episode's
// art — the subscription guard 404s before any art work. The validation-error
// body (code "exists") distinguishes it from the "no artwork" 204.
#[tokio::test]
async fn test_art_hidden_from_non_subscriber() {
    let (_root, dbc, payload) = setup_test_db().await;
    let episode_id = payload.get("episode_id_1").unwrap().as_str().unwrap();
    let token = generate_jwt_token(&(i32::MAX - 500).to_string());
    let router = build_test_router(dbc);

    let response = router
        .oneshot(authed(
            "GET",
            format!("/api/v1/episodes/{episode_id}/art"),
            &token,
        ))
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
    assert_eq!(code, "exists", "non-subscriber must not read episode art");
}
