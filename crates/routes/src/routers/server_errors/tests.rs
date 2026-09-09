use crate::routers::playbacks::tests::{build_test_router, generate_jwt_token, setup_test_db};
use crate::tests::harness::json_body;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

async fn get_errors(router: &axum::Router, token: Option<&str>) -> axum::response::Response {
    let mut b = Request::builder()
        .method("GET")
        .uri("/api/v1/admin/server-errors");
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
async fn server_errors_require_authentication() {
    let (_root, dbc, _payload) = setup_test_db().await;
    let router = build_test_router(dbc);
    assert_eq!(
        get_errors(&router, None).await.status(),
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn server_errors_forbidden_for_non_admin() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);
    let user = generate_jwt_token(payload["user_id"].as_str().unwrap());
    assert_eq!(
        get_errors(&router, Some(&user)).await.status(),
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn server_errors_admin_reads_persisted_rows() {
    let (_root, dbc, payload) = setup_test_db().await;

    // Seed one error of each kind against the fixture's podcast + episode.
    let podcast_id = payload["podcast_id"]
        .as_str()
        .unwrap()
        .parse::<i32>()
        .unwrap();
    let episode_id = payload["episode_id"]
        .as_str()
        .unwrap()
        .parse::<i32>()
        .unwrap();
    halogen_orm::podcast_sync_error::Entity::record(
        &dbc,
        podcast_id,
        "Failed to fetch feed: connection refused".to_string(),
    )
    .await
    .expect("record sync error");
    halogen_orm::episode_download_error::Entity::record(
        &dbc,
        episode_id,
        "origin returned 404 (not found)".to_string(),
    )
    .await
    .expect("record download error");

    let router = build_test_router(dbc);
    let admin = generate_jwt_token(payload["admin_id"].as_str().unwrap());
    let resp = get_errors(&router, Some(&admin)).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_body(resp).await;

    let rss = json["data"]["rss_sync"].as_array().expect("rss array");
    assert_eq!(rss.len(), 1);
    assert_eq!(rss[0]["podcast_id"].as_i64(), Some(podcast_id as i64));
    assert!(rss[0]["podcast_title"].is_string(), "title resolved");
    assert!(
        rss[0]["reason"]
            .as_str()
            .unwrap()
            .contains("connection refused")
    );

    let dls = json["data"]["episode_downloads"]
        .as_array()
        .expect("downloads array");
    assert_eq!(dls.len(), 1);
    assert_eq!(dls[0]["episode_id"].as_i64(), Some(episode_id as i64));
    assert!(dls[0]["episode_title"].is_string(), "title resolved");
    assert!(dls[0]["reason"].as_str().unwrap().contains("404"));
}
