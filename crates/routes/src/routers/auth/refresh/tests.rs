use std::time::Duration;

use axum::http::{StatusCode, header};
use tower::ServiceExt;

use super::super::super::tests::{build_test_router, setup_test_db};
use crate::tests::harness::{field_error, json, json_body};

#[tokio::test]
async fn test_refresh_returns_new_token() {
    let (_root, dbc, payload) = setup_test_db().await;
    let password = payload.get("password").unwrap().as_str().unwrap();
    let router = build_test_router(dbc.clone());

    // Login to get a token
    let login_body =
        serde_json::json!({ "data": { "username": "admin_user", "password": password } });
    let response = router
        .clone()
        .oneshot(json("POST", "/api/v1/auth/login", &login_body))
        .await
        .unwrap();

    let json_resp = json_body(response).await;
    let old_token = json_resp
        .get("data")
        .and_then(|d| d.get("token"))
        .unwrap()
        .as_str()
        .unwrap()
        .to_string();

    // Wait so the new token has a different expiration
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Refresh
    let refresh_body = serde_json::json!({ "data": { "token": old_token } });
    let response = router
        .clone()
        .oneshot(json("POST", "/api/v1/auth/refresh", &refresh_body))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    // Refresh re-issues the media cookie too.
    let cookie = response
        .headers()
        .get(header::SET_COOKIE)
        .expect("refresh must re-set the cookie")
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        cookie.starts_with("auth_media="),
        "cookie re-set on refresh"
    );

    let json_resp = json_body(response).await;
    let new_token = json_resp
        .get("data")
        .and_then(|d| d.get("token"))
        .unwrap()
        .as_str()
        .unwrap()
        .to_string();
    assert!(!new_token.is_empty(), "new token must not be empty");
    assert_ne!(new_token, old_token, "new token must differ from old one");
}

/// A WebSocket ticket (scope = "ws") must NOT be refreshable into a full
/// API token — the ws ticket rides in a URL query string and is scope-locked
/// to the `/ws` upgrade. Regression for the ws→API scope-confusion escalation.
#[tokio::test]
async fn test_refresh_ws_ticket_rejected() {
    use crate::routers::middleware::JwtClaims;
    use jsonwebtoken::{EncodingKey, Header, encode};

    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    // ws ticket for the real seeded admin, signed with the test secret.
    let sub = payload.get("id").unwrap().as_i64().unwrap().to_string();
    let ticket = encode(
        &Header::default(),
        &JwtClaims::ws(sub, 4_000_000_000),
        &EncodingKey::from_secret(b"test-secret-key-for-jwt-signing"),
    )
    .expect("encode ws ticket");

    let body = serde_json::json!({ "data": { "token": ticket } });
    let response = router
        .oneshot(json("POST", "/api/v1/auth/refresh", &body))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

/// A correctly-signed, unexpired API token whose subject no longer exists must
/// not be refreshable into a fresh session (a deleted user can't keep minting
/// tokens). Regression for the missing existence check.
#[tokio::test]
async fn test_refresh_unknown_subject_rejected() {
    use crate::routers::middleware::JwtClaims;
    use jsonwebtoken::{EncodingKey, Header, encode};

    let (_root, dbc, _payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    // Valid API token, but sub=1 was never seeded (admin is i32::MAX).
    let token = encode(
        &Header::default(),
        &JwtClaims::api("1".to_string(), 4_000_000_000),
        &EncodingKey::from_secret(b"test-secret-key-for-jwt-signing"),
    )
    .expect("encode api token");

    let body = serde_json::json!({ "data": { "token": token } });
    let response = router
        .oneshot(json("POST", "/api/v1/auth/refresh", &body))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_refresh_invalid_token_401() {
    let (_root, dbc, _meta) = setup_test_db().await;
    let router = build_test_router(dbc);

    let body = serde_json::json!({ "data": { "token": "bad.token.value" } });
    let response = router
        .clone()
        .oneshot(json("POST", "/api/v1/auth/refresh", &body))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_refresh_missing_data_field_400() {
    let (_root, dbc, _meta) = setup_test_db().await;
    let router = build_test_router(dbc);

    let body = serde_json::json!({});
    let response = router
        .clone()
        .oneshot(json("POST", "/api/v1/auth/refresh", &body))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = json_body(response).await;
    assert!(
        json.get("errors").is_some(),
        "error response must have errors envelope"
    );
    assert_eq!(field_error(&json, "data"), "Request body is required");
}

#[tokio::test]
async fn test_refresh_empty_token_400() {
    let (_root, dbc, _meta) = setup_test_db().await;
    let router = build_test_router(dbc);

    let body = serde_json::json!({ "data": { "token": "" } });
    let response = router
        .clone()
        .oneshot(json("POST", "/api/v1/auth/refresh", &body))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = json_body(response).await;
    assert!(
        json.get("errors").is_some(),
        "error response must have errors envelope"
    );
    assert!(
        json["errors"]["token"].is_array(),
        "validation errors should be keyed by field name"
    );
    assert_eq!(field_error(&json, "token"), "Token is required");
}

#[tokio::test]
async fn test_refresh_null_data_400() {
    let (_root, dbc, _meta) = setup_test_db().await;
    let router = build_test_router(dbc);

    let body = serde_json::json!({ "data": null });
    let response = router
        .clone()
        .oneshot(json("POST", "/api/v1/auth/refresh", &body))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = json_body(response).await;
    assert!(
        json.get("errors").is_some(),
        "error response must have errors envelope"
    );
    assert_eq!(field_error(&json, "data"), "Request body is required");
}

#[tokio::test]
async fn test_refresh_error_has_correct_envelope_shape() {
    let (_root, dbc, _meta) = setup_test_db().await;
    let router = build_test_router(dbc);

    let body = serde_json::json!({ "data": { "token": "not.a.real.token" } });
    let response = router
        .clone()
        .oneshot(json("POST", "/api/v1/auth/refresh", &body))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let json = json_body(response).await;

    assert!(json.get("data").is_some(), "response must have data field");
    assert!(
        json.get("errors").is_some(),
        "response must have errors field"
    );
    assert!(
        json.get("paginator").is_some(),
        "response must have paginator field"
    );

    assert!(
        json["errors"]["request"].as_array().is_some(),
        "errors should have 'request' key"
    );
    let errors = json["errors"]["request"].as_array().unwrap();
    assert_eq!(errors.len(), 1, "must have exactly one error");
    assert_eq!(
        errors[0]["code"].as_str().unwrap(),
        "unauthenticated",
        "error code must be 'unauthenticated'"
    );
    assert_eq!(
        errors[0]["message"].as_str().unwrap(),
        "Invalid or expired token",
        "error message must be 'Invalid or expired token'"
    );
}

#[tokio::test]
async fn test_refresh_database_outage_is_retryable() {
    use crate::routers::middleware::JwtClaims;
    use jsonwebtoken::{EncodingKey, Header, encode};

    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc.clone());
    let token = encode(
        &Header::default(),
        &JwtClaims::api(payload["id"].as_i64().unwrap().to_string(), 4_000_000_000),
        &EncodingKey::from_secret(b"test-secret-key-for-jwt-signing"),
    )
    .unwrap();
    dbc.close().await.unwrap();

    let response = router
        .oneshot(json(
            "POST",
            "/api/v1/auth/refresh",
            &serde_json::json!({ "data": { "token": token } }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = json_body(response).await;
    assert_eq!(body["errors"]["database"][0]["code"], "panic");
    assert_eq!(
        body["errors"]["database"][0]["message"],
        "Authentication is temporarily unavailable"
    );
}
