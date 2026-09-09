use crate::routers::playlists::tests::{build_test_router, generate_jwt_token, setup_test_db};
use crate::tests::harness::authed_json;
use axum::http::StatusCode;
use halogen_orm::episode_playlist::{Column, Entity as EpEntity};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use tower::ServiceExt;

fn bulk_body(ids: &[i32]) -> serde_json::Value {
    serde_json::json!({ "data": { "episode_ids": ids } })
}

async fn member(dbc: &sea_orm::DatabaseConnection, playlist_id: i32, episode_id: i32) -> bool {
    EpEntity::find()
        .filter(Column::PlaylistId.eq(playlist_id))
        .filter(Column::EpisodeId.eq(episode_id))
        .one(dbc)
        .await
        .unwrap()
        .is_some()
}

async fn member_count(dbc: &sea_orm::DatabaseConnection, playlist_id: i32) -> usize {
    EpEntity::find()
        .filter(Column::PlaylistId.eq(playlist_id))
        .all(dbc)
        .await
        .unwrap()
        .len()
}

fn ids(payload: &serde_json::Value) -> (i32, i32, i32) {
    // playlist_id_2 is owned by the user and starts empty (see membership tests).
    let pl: i32 = payload["playlist_id_2"].as_str().unwrap().parse().unwrap();
    let ep1: i32 = payload["episode_id_1"].as_str().unwrap().parse().unwrap();
    let ep2: i32 = payload["episode_id_2"].as_str().unwrap().parse().unwrap();
    (pl, ep1, ep2)
}

// POST bulk adds every authorized id (and is idempotent on a re-add).
#[tokio::test]
async fn test_bulk_store_adds_all_and_is_idempotent() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc.clone());
    let token = generate_jwt_token(payload["user_id"].as_str().unwrap());
    let (pl, ep1, ep2) = ids(&payload);

    let post = || {
        authed_json(
            "POST",
            format!("/api/v1/playlists/{pl}/episodes/bulk"),
            &token,
            &bulk_body(&[ep1, ep2]),
        )
    };

    let first = router.clone().oneshot(post()).await.unwrap();
    assert_eq!(first.status(), StatusCode::OK);
    assert!(member(&dbc, pl, ep1).await);
    assert!(member(&dbc, pl, ep2).await);

    // Re-adding the same ids is idempotent — still 200, no duplicate rows.
    let second = router.clone().oneshot(post()).await.unwrap();
    assert_eq!(second.status(), StatusCode::OK);
    assert_eq!(member_count(&dbc, pl).await, 2);
}

// An id the actor can't act on (here: a non-existent episode, which the
// subscription guard 404s exactly like a cross-user one) is filtered out — the
// authorized id still lands, the batch still succeeds (one bad id ≠ batch fail).
#[tokio::test]
async fn test_bulk_store_skips_unauthorized_id() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc.clone());
    let token = generate_jwt_token(payload["user_id"].as_str().unwrap());
    let (pl, _ep1, ep2) = ids(&payload);
    let bogus = 999_999;

    let response = router
        .oneshot(authed_json(
            "POST",
            format!("/api/v1/playlists/{pl}/episodes/bulk"),
            &token,
            &bulk_body(&[ep2, bogus]),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert!(member(&dbc, pl, ep2).await, "authorized id added");
    assert!(!member(&dbc, pl, bogus).await, "unauthorized id skipped");
    assert_eq!(member_count(&dbc, pl).await, 1);
}

// DELETE bulk removes members and is lenient about non-members in the same list.
#[tokio::test]
async fn test_bulk_delete_removes_members_lenient() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc.clone());
    let token = generate_jwt_token(payload["user_id"].as_str().unwrap());
    let (pl, ep1, ep2) = ids(&payload);

    // Seed: add only ep1 to the (empty) playlist.
    let add = router
        .clone()
        .oneshot(authed_json(
            "POST",
            format!("/api/v1/playlists/{pl}/episodes/bulk"),
            &token,
            &bulk_body(&[ep1]),
        ))
        .await
        .unwrap();
    assert_eq!(add.status(), StatusCode::OK);

    // Delete [ep1 (member), ep2 (NOT a member)] → 200; ep1 gone, ep2 no-op.
    let del = router
        .clone()
        .oneshot(authed_json(
            "DELETE",
            format!("/api/v1/playlists/{pl}/episodes/bulk"),
            &token,
            &bulk_body(&[ep1, ep2]),
        ))
        .await
        .unwrap();
    assert_eq!(del.status(), StatusCode::OK);
    assert!(!member(&dbc, pl, ep1).await, "member removed");
    assert_eq!(member_count(&dbc, pl).await, 0);
}

// Bulk remove from a playlist with `on_remove_delete_file_server` set: an
// episode whose LAST membership was removed loses its server file; one still
// held by another playlist keeps it.
#[tokio::test]
async fn test_bulk_delete_with_server_flag_deletes_files() {
    use halogen_orm::episode::ActiveModel as EpisodeActiveModel;
    use halogen_orm::playlist::ActiveModel as PlaylistActiveModel;
    use halogen_wire::DownloadStatus;
    use sea_orm::ActiveModelTrait;
    use sea_orm::ActiveValue::Set;

    let (root, dbc, payload) = setup_test_db().await;
    let token = generate_jwt_token(payload["user_id"].as_str().unwrap());
    // pl (playlist_id_2) starts empty; ep1 is also a seeded member of playlist_id_1.
    let (pl, ep1, ep2) = ids(&payload);

    PlaylistActiveModel {
        id: Set(pl),
        on_remove_delete_file_server: Set(true),
        ..Default::default()
    }
    .update(&dbc)
    .await
    .expect("set delete-on-remove flag");

    // Stage a downloaded file for both episodes.
    let mut paths = Vec::new();
    for (i, ep) in [ep1, ep2].into_iter().enumerate() {
        let file_path = root.path().join(format!("ep{i}.mp3"));
        std::fs::write(&file_path, b"AUDIO").expect("write temp audio");
        EpisodeActiveModel {
            id: Set(ep),
            download_status: Set(DownloadStatus::Downloaded),
            content_file_path: Set(Some(file_path.to_string_lossy().to_string())),
            ..Default::default()
        }
        .update(&dbc)
        .await
        .expect("mark downloaded");
        paths.push(file_path);
    }

    let router = build_test_router(dbc.clone());
    let add = router
        .clone()
        .oneshot(authed_json(
            "POST",
            format!("/api/v1/playlists/{pl}/episodes/bulk"),
            &token,
            &bulk_body(&[ep1, ep2]),
        ))
        .await
        .unwrap();
    assert_eq!(add.status(), StatusCode::OK);

    let del = router
        .clone()
        .oneshot(authed_json(
            "DELETE",
            format!("/api/v1/playlists/{pl}/episodes/bulk"),
            &token,
            &bulk_body(&[ep1, ep2]),
        ))
        .await
        .unwrap();
    assert_eq!(del.status(), StatusCode::OK);

    assert!(
        paths[0].exists(),
        "ep1 is still a member of another playlist — file must survive"
    );
    assert!(
        !paths[1].exists(),
        "ep2's last membership was removed — file must be deleted"
    );
}

// An empty id list fails the DTO validation in the `Body` extractor → 400.
#[tokio::test]
async fn test_bulk_store_empty_is_400() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);
    let token = generate_jwt_token(payload["user_id"].as_str().unwrap());
    let (pl, _ep1, _ep2) = ids(&payload);

    let response = router
        .oneshot(authed_json(
            "POST",
            format!("/api/v1/playlists/{pl}/episodes/bulk"),
            &token,
            &bulk_body(&[]),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}
