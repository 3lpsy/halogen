use crate::routers::playlists::tests::{build_test_router, generate_jwt_token, setup_test_db};
use crate::tests::harness::{authed_json, field_error, json_body};
use axum::http::StatusCode;
use tower::ServiceExt;

#[tokio::test]
async fn test_update_playlist_success() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let playlist_id = payload.get("playlist_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(user_id);

    let update_body = serde_json::json!({
        "data": {
            "name": "Updated Playlist Name"
        }
    });

    let response = router
        .clone()
        .oneshot(authed_json(
            "PUT",
            format!("/api/v1/playlists/{}", playlist_id),
            &token,
            &update_body,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let json = json_body(response).await;
    let pl_data = json.get("data").expect("data field");
    assert_eq!(
        pl_data.get("name").unwrap().as_str().unwrap(),
        "Updated Playlist Name"
    );
}

#[tokio::test]
async fn test_update_playlist_not_found() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(user_id);

    let fake_id = i32::MAX.to_string();

    let update_body = serde_json::json!({
        "data": {
            "name": "New Name"
        }
    });

    let response = router
        .clone()
        .oneshot(authed_json(
            "PUT",
            format!("/api/v1/playlists/{}", fake_id),
            &token,
            &update_body,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let json = json_body(response).await;
    assert_eq!(field_error(&json, "id"), "Playlist not found");
}

#[tokio::test]
async fn test_update_playlist_missing_body() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let playlist_id = payload.get("playlist_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(user_id);

    let update_body = serde_json::json!({});
    let response = router
        .clone()
        .oneshot(authed_json(
            "PUT",
            format!("/api/v1/playlists/{}", playlist_id),
            &token,
            &update_body,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let json = json_body(response).await;
    assert_eq!(field_error(&json, "data"), "Request body is required");
}

#[tokio::test]
async fn test_update_playlist_empty_name() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let playlist_id = payload.get("playlist_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(user_id);

    let update_body = serde_json::json!({
        "data": {
            "name": ""
        }
    });

    let response = router
        .clone()
        .oneshot(authed_json(
            "PUT",
            format!("/api/v1/playlists/{}", playlist_id),
            &token,
            &update_body,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let json = json_body(response).await;
    assert_eq!(field_error(&json, "name"), "Playlist name is required");
}
