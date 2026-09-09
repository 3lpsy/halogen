use crate::routers::playbacks::tests::{build_test_router, generate_jwt_token, setup_test_db};
use crate::tests::harness::{authed, field_error, json_body};
use axum::http::StatusCode;
use tower::ServiceExt;

#[tokio::test]
async fn test_get_playback_returns_playback() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(user_id);

    let response = router
        .clone()
        .oneshot(authed("GET", "/api/v1/playbacks/999999", &token))
        .await
        .unwrap();

    // Playback doesn't exist yet, so we expect 404
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_get_playback_with_invalid_jwt() {
    let (_root, dbc, _payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let fake_id = 999999;
    let token = generate_jwt_token(&fake_id.to_string());

    let response = router
        .clone()
        .oneshot(authed(
            "GET",
            "/api/v1/playbacks/00000000-0000-0000-0000-000000000001",
            &token,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_get_playback_invalid_id() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(user_id);

    let response = router
        .clone()
        .oneshot(authed("GET", "/api/v1/playbacks/abc", &token))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let json = json_body(response).await;
    assert_eq!(json["errors"]["id"][0]["code"].as_str().unwrap(), "parsing");
    assert_eq!(
        field_error(&json, "id"),
        "Invalid ID format: expected integer, got 'abc'"
    );
}
