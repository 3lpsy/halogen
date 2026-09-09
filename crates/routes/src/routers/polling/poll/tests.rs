use crate::routers::playbacks::tests::{build_test_router, generate_jwt_token, setup_test_db};
use crate::tests::harness::json_body;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

async fn poll(router: &axum::Router, token: Option<&str>) -> axum::response::Response {
    let mut b = Request::builder().method("POST").uri("/api/v1/admin/poll");
    if let Some(t) = token {
        b = b.header("Authorization", format!("Bearer {t}"));
    }
    router
        .clone()
        .oneshot(b.body(Body::empty()).unwrap())
        .await
        .unwrap()
}

#[tokio::test]
async fn poll_requires_authentication() {
    let (_root, dbc, _payload) = setup_test_db().await;
    let router = build_test_router(dbc);
    assert_eq!(poll(&router, None).await.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn poll_forbidden_for_non_admin() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);
    let user = generate_jwt_token(payload["user_id"].as_str().unwrap());
    assert_eq!(
        poll(&router, Some(&user)).await.status(),
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn poll_admin_succeeds() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);
    let admin = generate_jwt_token(payload["admin_id"].as_str().unwrap());
    let resp = poll(&router, Some(&admin)).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;
    assert_eq!(
        json["data"]["message"].as_str(),
        Some("Poll completed successfully")
    );
}
