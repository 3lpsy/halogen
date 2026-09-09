use crate::routers::podcasts::tests::{build_test_router, generate_jwt_token, setup_test_db};
use crate::tests::harness::{authed_json, field_error, json_body};
use axum::http::StatusCode;
use tower::ServiceExt;

#[tokio::test]
async fn test_update_podcast_config_success() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    // The seeded config is standalone (no podcast references it), so config
    // ownership is undecidable; owners/admins manage configs, so act as admin.
    let admin_id = payload.get("admin_id").unwrap().as_str().unwrap();
    let config_id = payload.get("podcast_config_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(admin_id);

    let body = serde_json::json!({ "data": { "id": 0, "poll_interval_seconds": 600 } });
    let response = router
        .clone()
        .oneshot(authed_json(
            "PUT",
            format!("/api/v1/podcast-configs/{}", config_id),
            &token,
            &body,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_update_podcast_config_not_found() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let fake_id = i32::MAX.to_string();
    let token = generate_jwt_token(user_id);

    let body = serde_json::json!({ "data": { "id": 0, "poll_interval_seconds": 600 } });
    let response = router
        .clone()
        .oneshot(authed_json(
            "PUT",
            format!("/api/v1/podcast-configs/{}", fake_id),
            &token,
            &body,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_update_podcast_config_invalid_id() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(user_id);

    let body = serde_json::json!({ "data": { "id": 0, "poll_interval_seconds": 600 } });
    let response = router
        .clone()
        .oneshot(authed_json(
            "PUT",
            "/api/v1/podcast-configs/abc",
            &token,
            &body,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_update_podcast_config_poll_interval_too_high() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    // Admin bypasses the ownership guard so the request reaches validation.
    let admin_id = payload.get("admin_id").unwrap().as_str().unwrap();
    let config_id = payload.get("podcast_config_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(admin_id);

    let body = serde_json::json!({ "data": { "id": 0, "poll_interval_seconds": 86401 } });
    let response = router
        .clone()
        .oneshot(authed_json(
            "PUT",
            format!("/api/v1/podcast-configs/{}", config_id),
            &token,
            &body,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let json = json_body(response).await;
    assert!(
        json.get("errors").is_some(),
        "error response must have errors envelope"
    );
    assert!(
        json["errors"]["poll_interval_seconds"].is_array(),
        "validation errors should be keyed by field name"
    );
    assert_eq!(
        field_error(&json, "poll_interval_seconds"),
        "Poll interval must be between 0 and 86400 seconds"
    );
}

#[tokio::test]
async fn test_update_podcast_config_max_episodes_too_high() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    // Admin bypasses the ownership guard so the request reaches validation.
    let admin_id = payload.get("admin_id").unwrap().as_str().unwrap();
    let config_id = payload.get("podcast_config_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(admin_id);

    let body = serde_json::json!({ "data": { "id": 0, "max_episodes": 10001 } });
    let response = router
        .clone()
        .oneshot(authed_json(
            "PUT",
            format!("/api/v1/podcast-configs/{}", config_id),
            &token,
            &body,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let json = json_body(response).await;
    assert!(
        json.get("errors").is_some(),
        "error response must have errors envelope"
    );
    assert!(
        json["errors"]["max_episodes"].is_array(),
        "validation errors should be keyed by field name"
    );
    assert_eq!(
        field_error(&json, "max_episodes"),
        "Max episodes must be between 1 and 10000"
    );
}

#[tokio::test]
async fn test_update_podcast_config_max_concurrent_downloads_zero() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    // Admin bypasses the ownership guard so the request reaches validation.
    let admin_id = payload.get("admin_id").unwrap().as_str().unwrap();
    let config_id = payload.get("podcast_config_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(admin_id);

    let body = serde_json::json!({ "data": { "id": 0, "max_concurrent_downloads": 0 } });
    let response = router
        .clone()
        .oneshot(authed_json(
            "PUT",
            format!("/api/v1/podcast-configs/{}", config_id),
            &token,
            &body,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let json = json_body(response).await;
    assert!(
        json.get("errors").is_some(),
        "error response must have errors envelope"
    );
    assert!(
        json["errors"]["max_concurrent_downloads"].is_array(),
        "validation errors should be keyed by field name"
    );
    assert_eq!(
        field_error(&json, "max_concurrent_downloads"),
        "Max concurrent downloads must be between 1 and 100"
    );
}
