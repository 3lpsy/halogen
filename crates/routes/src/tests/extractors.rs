use crate::routers::playbacks::tests::{build_test_router, generate_jwt_token, setup_test_db};
use crate::tests::harness::{authed, authed_json, field_error, json_body};
use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

/// `Id`: a non-integer path segment fails the `i32::parse` and yields a
/// 400 carrying the "Invalid ID format" message. We drive it through the
/// real `GET /users/{id}` route with a valid JWT so the middleware passes
/// and the `Id` extractor is the thing that rejects.
#[tokio::test]
async fn test_id_non_integer_path_is_400() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(user_id);

    let response = router
        .oneshot(authed("GET", "/api/v1/users/not-an-int", &token))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = json_body(response).await;
    // The id-extractor rejection is keyed under "id" (the location), with a
    // `parsing` reason for an unparseable segment.
    let message = field_error(&json, "id");
    assert!(
        message.contains("Invalid ID format"),
        "expected 'Invalid ID format' message, got: {message}"
    );
}

/// `Ids2`: a non-integer in either segment of `/playlists/{id}/episodes/{episode_id}`
/// fails `Path::<(i32, i32)>` and yields a 400 with the two-integers message.
#[tokio::test]
async fn test_ids2_non_integer_segment_is_400() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(user_id);

    // DELETE uses `Ids2`; the second segment is non-integer.
    let response = router
        .oneshot(authed(
            "DELETE",
            "/api/v1/playlists/1/episodes/not-an-int",
            &token,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = json_body(response).await;
    let message = field_error(&json, "id");
    assert!(
        message.contains("expected two integers"),
        "expected the two-integers message, got: {message}"
    );
}

/// `Body<T>`: a syntactically valid `RequestData` whose `data` is absent
/// (`{}` body) is rejected by the extractor with the exact
/// "Request body is required" message. Driven through `POST /podcasts`,
/// which uses `Body<PodcastStoreData>`.
#[tokio::test]
async fn test_body_missing_data_is_400() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(user_id);

    let body = serde_json::json!({});
    let response = router
        .oneshot(authed_json("POST", "/api/v1/podcasts", &token, &body))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = json_body(response).await;
    assert_eq!(field_error(&json, "data"), "Request body is required");
}

/// `Body<T>`: an unparseable JSON body trips the earlier `Json::from_request`
/// failure branch → "Failed to parse request body" (distinct from the
/// missing-`data` message above).
#[tokio::test]
async fn test_body_unparseable_json_is_400() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(user_id);

    let response = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/podcasts")
                .header("Authorization", format!("Bearer {token}"))
                .header("Content-Type", "application/json")
                .body(Body::from("{ this is not json"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = json_body(response).await;
    assert_eq!(field_error(&json, "data"), "Failed to parse request body");
}

/// `AuthUserId`: a fully valid request reaches the handler, confirming the
/// extractor read the id the middleware inserted (this is also the positive
/// middleware control). `GET /playbacks` returns 200 with an empty list.
#[tokio::test]
async fn test_auth_user_id_valid_request_is_200() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(user_id);

    let response = router
        .oneshot(authed("GET", "/api/v1/playbacks", &token))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}
