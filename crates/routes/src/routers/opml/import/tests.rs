use crate::routers::playbacks::tests::{build_test_router, generate_jwt_token, setup_test_db};
use crate::tests::harness::json_body;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use halogen_orm::podcast::{Column, Entity as PodcastEntity};
use halogen_wire::OpmlImportData;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use tower::ServiceExt;

const OPML: &str = r#"<?xml version="1.0"?>
<opml version="2.0"><body>
  <outline text="Imported Show" type="rss" xmlUrl="https://feeds.example.com/imported"/>
</body></opml>"#;

fn import_body() -> String {
    let body = halogen_wire::RequestData::<_, ()>::from_data(OpmlImportData {
        opml: OPML.to_string(),
    });
    serde_json::to_string(&body).unwrap()
}

async fn post_import(router: &axum::Router, token: Option<&str>) -> axum::response::Response {
    let mut b = Request::builder()
        .method("POST")
        .uri("/api/v1/admin/opml/import")
        .header("Content-Type", "application/json");
    if let Some(t) = token {
        b = b.header("Authorization", format!("Bearer {t}"));
    }
    router
        .clone()
        .oneshot(b.body(Body::from(import_body())).unwrap())
        .await
        .unwrap()
}

/// No token → 401 (the route is behind the JWT layer).
#[tokio::test]
async fn import_requires_authentication() {
    let (_root, dbc, _payload) = setup_test_db().await;
    let router = build_test_router(dbc);
    assert_eq!(
        post_import(&router, None).await.status(),
        StatusCode::UNAUTHORIZED
    );
}

/// A non-admin authed user → 403.
#[tokio::test]
async fn import_forbidden_for_non_admin() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);
    let user = generate_jwt_token(payload["user_id"].as_str().unwrap());
    assert_eq!(
        post_import(&router, Some(&user)).await.status(),
        StatusCode::FORBIDDEN
    );
}

/// Admin import inserts the podcast and reports the count.
#[tokio::test]
async fn import_admin_creates_podcast() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc.clone());
    let admin = generate_jwt_token(payload["admin_id"].as_str().unwrap());

    let resp = post_import(&router, Some(&admin)).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;
    assert_eq!(json["data"]["created"].as_u64(), Some(1));

    let exists = PodcastEntity::find()
        .filter(Column::FeedUrl.eq("https://feeds.example.com/imported"))
        .one(&dbc)
        .await
        .unwrap();
    assert!(exists.is_some(), "imported podcast row should exist");
}
