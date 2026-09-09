use crate::routers::playlists::tests::{build_test_router, generate_jwt_token, setup_test_db};
use crate::tests::harness::{authed, field_error, json_body};
use axum::http::StatusCode;
use tower::ServiceExt;

#[tokio::test]
async fn test_delete_playlist_success() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(user_id);

    let response = router
        .clone()
        .oneshot(authed(
            "DELETE",
            format!(
                "/api/v1/playlists/{}",
                payload.get("playlist_id").unwrap().as_str().unwrap()
            ),
            &token,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let json = json_body(response).await;
    let data = json.get("data").expect("data field");
    assert!(data.is_null());

    // Verify playlist is actually deleted
    let playlist_id = payload.get("playlist_id").unwrap().as_str().unwrap();
    let verify_response = router
        .clone()
        .oneshot(authed(
            "GET",
            format!("/api/v1/playlists/{}", playlist_id),
            &token,
        ))
        .await
        .unwrap();

    assert_eq!(verify_response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_delete_playlist_not_found() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let fake_id = i32::MAX.to_string();

    let token = generate_jwt_token(user_id);

    let response = router
        .clone()
        .oneshot(authed(
            "DELETE",
            format!("/api/v1/playlists/{}", fake_id),
            &token,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let json = json_body(response).await;
    let errors_array = json
        .get("errors")
        .unwrap()
        .get("id")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(
        errors_array[0].get("code").unwrap().as_str().unwrap(),
        "exists"
    );
    assert_eq!(field_error(&json, "id"), "Playlist not found");
}

#[tokio::test]
async fn test_delete_playlist_invalid_id() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(user_id);

    let response = router
        .clone()
        .oneshot(authed("DELETE", "/api/v1/playlists/abc", &token))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let json = json_body(response).await;
    let errors_array = json
        .get("errors")
        .unwrap()
        .get("id")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(
        errors_array[0].get("code").unwrap().as_str().unwrap(),
        "parsing"
    );
    assert_eq!(
        field_error(&json, "id"),
        "Invalid ID format: expected integer, got 'abc'"
    );
}
