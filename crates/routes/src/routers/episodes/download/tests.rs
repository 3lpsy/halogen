use crate::routers::episodes::tests::{build_test_router, generate_jwt_token, setup_test_db};
use crate::tests::harness::{authed, unauthed};
use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

// No Authorization header → the JWT layer rejects before the handler runs.
#[tokio::test]
async fn test_download_without_auth_is_unauthorized() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let episode_id = payload.get("episode_id_1").unwrap().as_str().unwrap();

    let response = router
        .clone()
        .oneshot(unauthed(
            "POST",
            format!("/api/v1/episodes/{}/download", episode_id),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

// A garbage bearer token fails JWT validation → 401.
#[tokio::test]
async fn test_download_with_invalid_token_is_unauthorized() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let episode_id = payload.get("episode_id_1").unwrap().as_str().unwrap();

    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/episodes/{}/download", episode_id))
                .header("Authorization", "Bearer not-a-valid-jwt")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

// A non-integer id is rejected by the `Id` extractor with a 400.
#[tokio::test]
async fn test_download_with_invalid_id() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(user_id);

    let response = router
        .clone()
        .oneshot(authed("POST", "/api/v1/episodes/abc/download", &token))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

// Valid token + valid episode id → 202 Accepted immediately; the real fetch
// runs in a spawned task, so we only assert the synchronous acknowledgement.
#[tokio::test]
async fn test_download_valid_returns_accepted() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(user_id);
    let episode_id = payload.get("episode_id_1").unwrap().as_str().unwrap();

    let response = router
        .clone()
        .oneshot(authed(
            "POST",
            format!("/api/v1/episodes/{}/download", episode_id),
            &token,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::ACCEPTED);
}

// DELETE without auth → 401 (JWT layer rejects before the handler).
#[tokio::test]
async fn test_remove_without_auth_is_unauthorized() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);
    let episode_id = payload.get("episode_id_1").unwrap().as_str().unwrap();

    let response = router
        .clone()
        .oneshot(unauthed(
            "DELETE",
            format!("/api/v1/episodes/{}/download", episode_id),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

// DELETE on a downloaded episode → 200, the file is deleted and the episode
// resets to `NotDownloaded` with no path.
#[tokio::test]
async fn test_remove_deletes_file_and_resets_status() {
    use halogen_orm::episode::{
        ActiveModel as EpisodeActiveModel, Column as EpisodeColumn, Entity as EpisodeEntity,
    };
    use halogen_wire::DownloadStatus;
    use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter};

    let (root, dbc, payload) = setup_test_db().await;
    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(user_id);
    let episode_id: i32 = payload
        .get("episode_id_1")
        .unwrap()
        .as_str()
        .unwrap()
        .parse()
        .unwrap();

    // Stage a real file and mark the episode downloaded.
    let file_path = root.path().join("ep.mp3");
    std::fs::write(&file_path, b"AUDIO").expect("write temp audio");
    let abs = file_path.to_string_lossy().to_string();
    let update = EpisodeActiveModel {
        id: sea_orm::ActiveValue::set(episode_id),
        download_status: sea_orm::ActiveValue::set(DownloadStatus::Downloaded),
        content_file_path: sea_orm::ActiveValue::set(Some(abs.clone())),
        ..Default::default()
    };
    update.update(&dbc).await.expect("mark downloaded");

    let router = build_test_router(dbc.clone());
    let response = router
        .clone()
        .oneshot(authed(
            "DELETE",
            format!("/api/v1/episodes/{}/download", episode_id),
            &token,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert!(!file_path.exists(), "the downloaded file must be deleted");

    let ep = EpisodeEntity::find()
        .filter(EpisodeColumn::Id.eq(episode_id))
        .one(&dbc)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(ep.download_status, DownloadStatus::NotDownloaded);
    assert!(ep.content_file_path.is_none(), "path cleared");
}
