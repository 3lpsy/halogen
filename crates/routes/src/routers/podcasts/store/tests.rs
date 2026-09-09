use crate::routers::podcasts::tests::{build_test_router, generate_jwt_token, setup_test_db};
use crate::tests::harness::{authed_json, field_error, json_body};
use axum::http::StatusCode;
use tower::ServiceExt;

#[tokio::test]
async fn test_store_podcast_success() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(user_id);

    let store_body = serde_json::json!({
        "data": {
            "title": "New Podcast",
            "description": "Podcast description",
            "feed_url": "https://example.com/new-feed.xml",
            "art_url": null,
            "author": null
        }
    });

    let response = router
        .clone()
        .oneshot(authed_json("POST", "/api/v1/podcasts", &token, &store_body))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let json = json_body(response).await;
    let ep = json.get("data").expect("data field");
    assert_eq!(ep.get("title").unwrap().as_str().unwrap(), "New Podcast");
    assert_eq!(
        ep.get("feed_url").unwrap().as_str().unwrap(),
        "https://example.com/new-feed.xml"
    );
}

#[tokio::test]
async fn test_store_podcast_missing_body() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(user_id);

    let body = serde_json::json!({});
    let response = router
        .clone()
        .oneshot(authed_json("POST", "/api/v1/podcasts", &token, &body))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let json = json_body(response).await;
    assert_eq!(field_error(&json, "data"), "Request body is required");
}

#[tokio::test]
async fn test_store_podcast_empty_title_400() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(user_id);

    let body = serde_json::json!({
        "data": {
            "title": "",
            "feed_url": "https://example.com/feed.xml"
        }
    });
    let response = router
        .clone()
        .oneshot(authed_json("POST", "/api/v1/podcasts", &token, &body))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = json_body(response).await;
    assert!(
        json["errors"]["title"].is_array(),
        "validation errors should be keyed by field name"
    );
    assert_eq!(
        field_error(&json, "title"),
        "Title must be between 1 and 256 characters long"
    );
}

#[tokio::test]
async fn test_store_podcast_invalid_feed_url_400() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(user_id);

    let body = serde_json::json!({
        "data": {
            "title": "Test Podcast",
            "feed_url": "not-a-valid-url"
        }
    });
    let response = router
        .clone()
        .oneshot(authed_json("POST", "/api/v1/podcasts", &token, &body))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = json_body(response).await;
    assert!(
        json["errors"]["feed_url"].is_array(),
        "validation errors should be keyed by field name"
    );
    assert_eq!(
        field_error(&json, "feed_url"),
        "Feed URL must be a valid URL"
    );
}

#[tokio::test]
async fn test_store_podcast_title_too_long_400() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(user_id);

    let body = serde_json::json!({
        "data": {
            "title": "a".repeat(257),
            "feed_url": "https://example.com/feed.xml"
        }
    });
    let response = router
        .clone()
        .oneshot(authed_json("POST", "/api/v1/podcasts", &token, &body))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = json_body(response).await;
    assert!(
        json["errors"]["title"].is_array(),
        "validation errors should be keyed by field name"
    );
    assert_eq!(
        field_error(&json, "title"),
        "Title must be between 1 and 256 characters long"
    );
}

#[tokio::test]
async fn test_store_podcast_invalid_art_url_400() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(user_id);

    let body = serde_json::json!({
        "data": {
            "title": "Test Podcast",
            "feed_url": "https://example.com/feed.xml",
            "art_url": "not-a-url"
        }
    });
    let response = router
        .clone()
        .oneshot(authed_json("POST", "/api/v1/podcasts", &token, &body))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = json_body(response).await;
    assert!(
        json["errors"]["art_url"].is_array(),
        "validation errors should be keyed by field name"
    );
    assert_eq!(field_error(&json, "art_url"), "Art URL must be a valid URL");
}
