use crate::routers::playbacks::tests::{build_test_router, generate_jwt_token, setup_test_db};
use crate::tests::harness::json_body;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use halogen_utils::opml::{extract_podcasts_from_opml, parse_opml_str};
use tower::ServiceExt;

async fn get_export(router: &axum::Router, token: Option<&str>) -> axum::response::Response {
    let mut b = Request::builder()
        .method("GET")
        .uri("/api/v1/admin/opml/export");
    if let Some(t) = token {
        b = b.header("Authorization", format!("Bearer {t}"));
    }
    router
        .clone()
        .oneshot(b.body(Body::empty()).unwrap())
        .await
        .unwrap()
}

/// No token → 401.
#[tokio::test]
async fn export_requires_authentication() {
    let (_root, dbc, _payload) = setup_test_db().await;
    let router = build_test_router(dbc);
    assert_eq!(
        get_export(&router, None).await.status(),
        StatusCode::UNAUTHORIZED
    );
}

/// A non-admin authed user → 403.
#[tokio::test]
async fn export_forbidden_for_non_admin() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);
    let user = generate_jwt_token(payload["user_id"].as_str().unwrap());
    assert_eq!(
        get_export(&router, Some(&user)).await.status(),
        StatusCode::FORBIDDEN
    );
}

/// Admin export returns OPML that parses back to the seeded subscription.
#[tokio::test]
async fn export_admin_returns_opml() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);
    let admin = generate_jwt_token(payload["admin_id"].as_str().unwrap());

    let resp = get_export(&router, Some(&admin)).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;
    let opml = json["data"]["opml"].as_str().expect("opml string");

    // The serialized OPML round-trips and carries the seeded podcast's feed.
    let doc = parse_opml_str(opml).expect("export should be valid OPML");
    let podcasts = extract_podcasts_from_opml(&doc);
    assert!(
        podcasts
            .iter()
            .any(|(_, url)| url == "https://example.com/feed.xml"),
        "exported OPML should include the seeded feed; got {podcasts:?}"
    );
}
