use crate::routers::episodes::tests::{build_test_router, generate_jwt_token, setup_test_db};
use crate::tests::harness::{authed, json_body};
use axum::http::StatusCode;
use tower::ServiceExt;

#[tokio::test]
async fn test_list_episodes_returns_all() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(user_id);

    let response = router
        .clone()
        .oneshot(authed("GET", "/api/v1/episodes", &token))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let json = json_body(response).await;
    let episodes = json
        .get("data")
        .and_then(|d| d.as_array())
        .expect("data is array");
    assert_eq!(episodes.len(), 2, "should return 2 seeded episodes");
}

#[tokio::test]
async fn test_list_episodes_includes_paginator() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(user_id);

    let response = router
        .clone()
        .oneshot(authed("GET", "/api/v1/episodes", &token))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let json = json_body(response).await;
    // `paginator` reports the page metadata.
    let p = &json["paginator"];
    assert!(!p.is_null(), "paginator must be populated");
    assert_eq!(p["page"].as_i64(), Some(0));
    assert_eq!(p["size"].as_i64(), Some(10), "default size");
    assert_eq!(p["total"].as_i64(), Some(2), "two seeded episodes");
    assert_eq!(p["pages"].as_i64(), Some(1));
}

#[tokio::test]
async fn test_list_episodes_with_invalid_jwt() {
    let (_root, dbc, _payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let fake_id = 999999;
    let token = generate_jwt_token(&fake_id.to_string());

    let response = router
        .clone()
        .oneshot(authed("GET", "/api/v1/episodes", &token))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_list_episodes_invalid_page_400() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(user_id);

    let response = router
        .clone()
        .oneshot(authed(
            "GET",
            "/api/v1/episodes?pagination[page]=-1&pagination[size]=10",
            &token,
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
        json["errors"]["pagination.page"].is_array(),
        "validation errors should be keyed by field name"
    );
}

#[tokio::test]
async fn test_list_episodes_invalid_size_400() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(user_id);

    let response = router
        .clone()
        .oneshot(authed(
            "GET",
            "/api/v1/episodes?pagination[page]=1&pagination[size]=0",
            &token,
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
        json["errors"]["pagination.size"].is_array(),
        "validation errors should be keyed by field name"
    );
}

#[tokio::test]
async fn test_list_episodes_too_many_includes_400() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(user_id);

    let response = router
        .clone()
        .oneshot(authed(
            "GET",
            "/api/v1/episodes?includes[]=Podcast&includes[]=Podcast&includes[]=Podcast&includes[]=Podcast&includes[]=Podcast&includes[]=Podcast&includes[]=Podcast&includes[]=Podcast&includes[]=Podcast&includes[]=Podcast&includes[]=Podcast",
            &token,
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
        json["errors"]["includes"].is_array(),
        "validation errors should be keyed by field name"
    );
}

#[tokio::test]
async fn test_list_episodes_with_valid_include() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(user_id);

    let response = router
        .clone()
        .oneshot(authed("GET", "/api/v1/episodes?includes[]=Podcast", &token))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let json = json_body(response).await;
    let episodes = json
        .get("data")
        .and_then(|d| d.as_array())
        .expect("data is array");
    assert_eq!(episodes.len(), 2, "should return 2 seeded episodes");
}
