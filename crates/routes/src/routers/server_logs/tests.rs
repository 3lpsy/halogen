use crate::routers::playbacks::tests::{build_test_router, generate_jwt_token, setup_test_db};
use crate::tests::harness::json_body;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

async fn get_logs(router: &axum::Router, token: Option<&str>) -> axum::response::Response {
    let mut b = Request::builder()
        .method("GET")
        .uri("/api/v1/admin/server-logs");
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
async fn server_logs_require_authentication() {
    let (_root, dbc, _payload) = setup_test_db().await;
    let router = build_test_router(dbc);
    assert_eq!(
        get_logs(&router, None).await.status(),
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn server_logs_forbidden_for_non_admin() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);
    let user = generate_jwt_token(payload["user_id"].as_str().unwrap());
    assert_eq!(
        get_logs(&router, Some(&user)).await.status(),
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn server_logs_admin_gets_memory_ring_when_no_file_configured() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);
    let admin = generate_jwt_token(payload["admin_id"].as_str().unwrap());
    // No file configured → served from the in-memory ring (`path: null`).
    // The test binary never installs the tracing subscriber, so the ring is
    // empty here — the fallback wiring itself is covered by the ring unit
    // tests in `crate::logging`.
    let resp = get_logs(&router, Some(&admin)).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;
    assert!(json["data"]["path"].is_null());
    assert_eq!(json["data"]["lines"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn tail_file_returns_last_lines_in_order() {
    let mut root = halogen_fixture::test_support::TestRoot::new("server_logs_tail");
    let path = root.path().join("halogen.log");
    let content: String = (1..=10).map(|i| format!("line {i}\n")).collect();
    std::fs::write(&path, content).unwrap();

    let tail = super::tail_file(&path, 3).await.unwrap();
    assert_eq!(tail, vec!["line 8", "line 9", "line 10"]);

    // Asking for more lines than exist returns the whole file.
    let all = super::tail_file(&path, 100).await.unwrap();
    assert_eq!(all.len(), 10);
    assert_eq!(all[0], "line 1");
    root.mark_success();
}
