use crate::routers::users::tests::{build_test_router, generate_jwt_token, setup_test_db};
use crate::tests::harness::{authed, field_error, json_body};
use axum::http::StatusCode;
use tower::ServiceExt;

#[tokio::test]
async fn test_list_users_returns_all_users() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let admin_id = payload.get("admin_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(admin_id);

    let response = router
        .clone()
        .oneshot(authed("GET", "/api/v1/admin/users", &token))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let json = json_body(response).await;
    let users = json
        .get("data")
        .and_then(|d| d.as_array())
        .expect("data is array");
    assert_eq!(users.len(), 2, "should return 2 seeded users");
}

#[tokio::test]
async fn test_list_users_with_invalid_jwt() {
    let (_root, dbc, _payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let fake_id = 999999;
    let token = generate_jwt_token(&fake_id.to_string());

    let response = router
        .clone()
        .oneshot(authed("GET", "/api/v1/admin/users", &token))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_list_users_invalid_page_400() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let admin_id = payload.get("admin_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(admin_id);

    let response = router
        .clone()
        .oneshot(authed(
            "GET",
            "/api/v1/admin/users?pagination[page]=-1&pagination[size]=10",
            &token,
        ))
        .await
        .unwrap();

    let status = response.status();
    let json = json_body(response).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    assert!(
        json.get("errors").is_some(),
        "error response must have errors envelope"
    );
    assert!(
        json["errors"]["pagination.page"].is_array(),
        "validation errors should be keyed by field name"
    );
    assert_eq!(
        field_error(&json, "pagination.page"),
        "Page must be between 0 and 65536"
    );
}

#[tokio::test]
async fn test_list_users_invalid_size_400() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let admin_id = payload.get("admin_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(admin_id);

    let response = router
        .clone()
        .oneshot(authed(
            "GET",
            "/api/v1/admin/users?pagination[page]=1&pagination[size]=0",
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
    assert_eq!(
        field_error(&json, "pagination.size"),
        "Size must be between 1 and 65536"
    );
}

#[tokio::test]
async fn test_list_users_page_too_large_400() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let admin_id = payload.get("admin_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(admin_id);

    let response = router
        .clone()
        .oneshot(authed(
            "GET",
            "/api/v1/admin/users?pagination[page]=65537&pagination[size]=10",
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
async fn test_list_users_size_too_large_400() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let admin_id = payload.get("admin_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(admin_id);

    let response = router
        .clone()
        .oneshot(authed(
            "GET",
            "/api/v1/admin/users?pagination[page]=1&pagination[size]=65537",
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
async fn test_list_users_valid_pagination() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let admin_id = payload.get("admin_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(admin_id);

    let response = router
        .clone()
        .oneshot(authed(
            "GET",
            "/api/v1/admin/users?pagination[page]=0&pagination[size]=10",
            &token,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let json = json_body(response).await;
    let users = json
        .get("data")
        .and_then(|d| d.as_array())
        .expect("data is array");
    assert_eq!(users.len(), 2, "should return 2 seeded users");
}

#[tokio::test]
async fn test_list_users_forbidden_for_non_admin() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    // Listing users is admin-only — a non-admin token → 403.
    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(user_id);

    let response = router
        .clone()
        .oneshot(authed("GET", "/api/v1/admin/users", &token))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}
