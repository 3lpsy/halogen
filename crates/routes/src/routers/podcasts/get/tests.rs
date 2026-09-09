use crate::routers::podcasts::tests::{build_test_router, generate_jwt_token, setup_test_db};
use crate::tests::harness::{authed, field_error, json_body};
use axum::http::StatusCode;
use tower::ServiceExt;

#[tokio::test]
async fn test_get_podcast_returns_podcast() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let podcast_id = payload.get("podcast_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(user_id);

    let response = router
        .clone()
        .oneshot(authed(
            "GET",
            format!("/api/v1/podcasts/{podcast_id}"),
            &token,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = json_body(response).await;
    assert_eq!(json["data"]["title"].as_str().unwrap(), "Test Podcast");
}

#[tokio::test]
async fn test_get_podcast_not_found() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(user_id);
    let fake_id = i32::MAX.to_string();

    let response = router
        .clone()
        .oneshot(authed("GET", format!("/api/v1/podcasts/{fake_id}"), &token))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let json = json_body(response).await;
    assert_eq!(field_error(&json, "id"), "Podcast not found");
}

#[tokio::test]
async fn test_get_podcast_invalid_id() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(user_id);

    let response = router
        .clone()
        .oneshot(authed("GET", "/api/v1/podcasts/abc", &token))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = json_body(response).await;
    assert_eq!(
        field_error(&json, "id"),
        "Invalid ID format: expected integer, got 'abc'"
    );
}
