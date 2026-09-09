use axum::http::{StatusCode, header};
use tower::ServiceExt;

use super::super::super::tests::{build_test_router, setup_test_db};
use crate::tests::harness::{json_body, unauthed};

#[tokio::test]
async fn test_logout_returns_ok() {
    let (_root, dbc, _meta) = setup_test_db().await;
    let router = build_test_router(dbc);

    let response = router
        .clone()
        .oneshot(unauthed("POST", "/api/v1/auth/logout"))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    // Logout expires the media cookie.
    let cookie = response
        .headers()
        .get(header::SET_COOKIE)
        .expect("logout must clear the cookie")
        .to_str()
        .unwrap()
        .to_string();
    assert!(cookie.starts_with("auth_media=;"), "cookie value cleared");
    assert!(cookie.contains("Max-Age=0"), "cookie expires immediately");

    let json = json_body(response).await;
    assert!(
        json.get("data").is_some(),
        "logout should return data field"
    );
}
