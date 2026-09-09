use crate::routers::users::tests::{build_test_router, generate_jwt_token, setup_test_db};
use crate::tests::harness::{authed_json, field_error, json_body};
use axum::http::StatusCode;
use tower::ServiceExt;

#[tokio::test]
async fn test_update_user_success() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(user_id);

    let update_body = serde_json::json!({
        "data": {
            "username": "updated_username"
        }
    });

    let response = router
        .clone()
        .oneshot(authed_json(
            "PUT",
            format!("/api/v1/users/{}", user_id),
            &token,
            &update_body,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let json = json_body(response).await;
    let user_data = json.get("data").expect("data field");
    assert_eq!(
        user_data.get("username").unwrap().as_str().unwrap(),
        "updated_username"
    );
}

/// Usernames are stored lowercase: a mixed-case submission is normalized
/// before the write and echoed back lowercased.
#[tokio::test]
async fn test_update_user_username_is_lowercased() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(user_id);

    let update_body = serde_json::json!({
        "data": {
            "username": "Updated_UserName"
        }
    });

    let response = router
        .clone()
        .oneshot(authed_json(
            "PUT",
            format!("/api/v1/users/{}", user_id),
            &token,
            &update_body,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let json = json_body(response).await;
    assert_eq!(
        json["data"]["username"].as_str().unwrap(),
        "updated_username"
    );
}

#[tokio::test]
async fn test_update_user_not_found() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let auth_id = payload.get("admin_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(auth_id);

    let fake_id = i32::MAX.to_string();

    let update_body = serde_json::json!({
        "data": {
            "username": "new_name"
        }
    });

    let response = router
        .clone()
        .oneshot(authed_json(
            "PUT",
            format!("/api/v1/users/{}", fake_id),
            &token,
            &update_body,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let json = json_body(response).await;
    assert_eq!(json["errors"]["id"][0]["code"].as_str().unwrap(), "exists");
    assert_eq!(field_error(&json, "id"), "user does not exist");
}

#[tokio::test]
async fn test_update_user_missing_body() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(user_id);

    let body = serde_json::json!({});
    let response = router
        .clone()
        .oneshot(authed_json(
            "PUT",
            format!("/api/v1/users/{}", user_id),
            &token,
            &body,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let json = json_body(response).await;
    assert_eq!(
        json["errors"]["data"][0]["code"].as_str().unwrap(),
        "invalid"
    );
    assert_eq!(field_error(&json, "data"), "Request body is required");
}

#[tokio::test]
async fn test_update_user_username_too_short() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(user_id);

    let update_body = serde_json::json!({
        "data": {
            "username": "ab"
        }
    });

    let response = router
        .clone()
        .oneshot(authed_json(
            "PUT",
            format!("/api/v1/users/{}", user_id),
            &token,
            &update_body,
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
        json["errors"]["username"].is_array(),
        "validation errors should be keyed by field name"
    );
    assert_eq!(
        field_error(&json, "username"),
        "Username must be between 3 and 64 characters long"
    );
}

#[tokio::test]
async fn test_update_user_username_too_long() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(user_id);

    let update_body = serde_json::json!({
        "data": {
            "username": "a".repeat(65)
        }
    });

    let response = router
        .clone()
        .oneshot(authed_json(
            "PUT",
            format!("/api/v1/users/{}", user_id),
            &token,
            &update_body,
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
        json["errors"]["username"].is_array(),
        "validation errors should be keyed by field name"
    );
    assert_eq!(
        field_error(&json, "username"),
        "Username must be between 3 and 64 characters long"
    );
}

#[tokio::test]
async fn test_update_other_user_forbidden_for_non_admin() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    // Non-admin updating a *different* user → 403.
    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let admin_id = payload.get("admin_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(user_id);

    let body = serde_json::json!({ "data": { "username": "hijacked_name" } });
    let response = router
        .clone()
        .oneshot(authed_json(
            "PUT",
            format!("/api/v1/users/{}", admin_id),
            &token,
            &body,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_self_update_cannot_grant_admin() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    // A non-admin updating their OWN record may not flip is_admin → 403
    // (the privilege-escalation vector this guard exists to close).
    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(user_id);

    let body = serde_json::json!({ "data": { "is_admin": true } });
    let response = router
        .clone()
        .oneshot(authed_json(
            "PUT",
            format!("/api/v1/users/{}", user_id),
            &token,
            &body,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_admin_can_set_admin_on_other_user() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    // An admin may grant admin to another user.
    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let admin_id = payload.get("admin_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(admin_id);

    let body = serde_json::json!({ "data": { "is_admin": true } });
    let response = router
        .clone()
        .oneshot(authed_json(
            "PUT",
            format!("/api/v1/users/{}", user_id),
            &token,
            &body,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let json = json_body(response).await;
    assert!(
        json["data"]["is_admin"].as_bool().unwrap(),
        "admin flag should be set on the target user"
    );
}
