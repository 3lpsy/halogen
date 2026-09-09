use crate::routers::podcasts::tests::{build_test_router, generate_jwt_token, setup_test_db};
use crate::tests::harness::{authed, authed_json, json_body};
use axum::http::StatusCode;
use tower::ServiceExt;

/// Create a config for a podcast that has none → 200, and the returned config
/// carries the submitted override.
#[tokio::test]
async fn test_store_config_for_podcast_success() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let podcast_id = payload.get("podcast_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(user_id);

    let body = serde_json::json!({ "data": { "poll_interval_seconds": 300 } });
    let response = router
        .clone()
        .oneshot(authed_json(
            "POST",
            format!("/api/v1/podcasts/{}/config", podcast_id),
            &token,
            &body,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = json_body(response).await;
    assert_eq!(json["data"]["poll_interval_seconds"], 300);
}

/// Creating a second config for a podcast that already has one → rejected.
#[tokio::test]
async fn test_store_config_for_podcast_already_exists() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let podcast_id = payload.get("podcast_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(user_id);

    let body = serde_json::json!({ "data": { "max_episodes": 10 } });
    let mk = || {
        authed_json(
            "POST",
            format!("/api/v1/podcasts/{}/config", podcast_id),
            &token,
            &body,
        )
    };

    let first = router.clone().oneshot(mk()).await.unwrap();
    assert_eq!(first.status(), StatusCode::OK);

    let second = router.clone().oneshot(mk()).await.unwrap();
    assert_eq!(second.status(), StatusCode::CONFLICT);
}

/// Create config for a missing podcast → 404.
#[tokio::test]
async fn test_store_config_for_missing_podcast_404() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(user_id);

    let body = serde_json::json!({ "data": { "max_episodes": 10 } });
    let response = router
        .clone()
        .oneshot(authed_json(
            "POST",
            "/api/v1/podcasts/123456/config",
            &token,
            &body,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

/// Out-of-range override → 400 (validated by the `Body` extractor).
#[tokio::test]
async fn test_store_config_for_podcast_invalid_400() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let podcast_id = payload.get("podcast_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(user_id);

    let body = serde_json::json!({ "data": { "poll_interval_seconds": 86401 } });
    let response = router
        .clone()
        .oneshot(authed_json(
            "POST",
            format!("/api/v1/podcasts/{}/config", podcast_id),
            &token,
            &body,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

/// Remove a config: create one, then DELETE → 200, and a second DELETE is an
/// idempotent no-op success.
#[tokio::test]
async fn test_remove_config_for_podcast() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let podcast_id = payload.get("podcast_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(user_id);

    let create_body = serde_json::json!({ "data": { "max_episodes": 10 } });
    let create = router
        .clone()
        .oneshot(authed_json(
            "POST",
            format!("/api/v1/podcasts/{}/config", podcast_id),
            &token,
            &create_body,
        ))
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::OK);

    let mk_delete = || {
        authed(
            "DELETE",
            format!("/api/v1/podcasts/{}/config", podcast_id),
            &token,
        )
    };

    let first = router.clone().oneshot(mk_delete()).await.unwrap();
    assert_eq!(first.status(), StatusCode::OK);

    let second = router.clone().oneshot(mk_delete()).await.unwrap();
    assert_eq!(second.status(), StatusCode::OK);
}
