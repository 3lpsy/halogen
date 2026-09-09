use crate::routers::playlists::tests::{build_test_router, generate_jwt_token, setup_test_db};
use crate::tests::harness::{authed, authed_json, field_error, json, json_body, unauthed};
use axum::http::StatusCode;
use tower::ServiceExt;

#[tokio::test]
async fn test_store_episode_playlist_success() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let playlist_id: i32 = payload
        .get("playlist_id_1")
        .unwrap()
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let episode_id: i32 = payload
        .get("episode_id_2")
        .unwrap()
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let token = generate_jwt_token(user_id);

    // The ids come from the path; the body carries only the optional position.
    let store_body = serde_json::json!({ "data": {} });

    let response = router
        .clone()
        .oneshot(authed_json(
            "POST",
            format!("/api/v1/playlists/{}/episodes/{}", playlist_id, episode_id),
            &token,
            &store_body,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let json = json_body(response).await;
    println!("Response: {}", json);
    let ep_pl = json.get("data").expect("data field");
    assert_eq!(
        ep_pl.get("playlist_id").unwrap().as_i64().unwrap() as i32,
        playlist_id
    );
}

#[tokio::test]
async fn test_store_episode_playlist_missing_body() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(user_id);
    // Use a real owned playlist + episode so the `Ids2` extractor and the
    // ownership guard both pass; the missing body is what must be rejected.
    let playlist_id = payload.get("playlist_id_1").unwrap().as_str().unwrap();
    let episode_id = payload.get("episode_id_2").unwrap().as_str().unwrap();

    let store_body = serde_json::json!({});
    let response = router
        .clone()
        .oneshot(authed_json(
            "POST",
            format!("/api/v1/playlists/{}/episodes/{}", playlist_id, episode_id),
            &token,
            &store_body,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = json_body(response).await;
    assert_eq!(field_error(&json, "data"), "Request body is required");
}

/// Adding the same episode to the same playlist twice is idempotent: a unique
/// (episode_id, playlist_id) index backs the table, and `handle_store` returns
/// the existing row instead of inserting a duplicate — so the second add still
/// succeeds (200) without creating a second membership.
#[tokio::test]
async fn test_store_episode_playlist_duplicate_is_idempotent() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let playlist_id: i32 = payload
        .get("playlist_id_2")
        .unwrap()
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let episode_id: i32 = payload
        .get("episode_id_2")
        .unwrap()
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let token = generate_jwt_token(user_id);

    let store_body = serde_json::json!({ "data": {} });

    let make_request = || {
        authed_json(
            "POST",
            format!("/api/v1/playlists/{}/episodes/{}", playlist_id, episode_id),
            &token,
            &store_body,
        )
    };

    // First insert succeeds.
    let first = router.clone().oneshot(make_request()).await.unwrap();
    assert_eq!(first.status(), StatusCode::OK);

    // Second identical insert is idempotent (returns the existing row) → also 200.
    let second = router.clone().oneshot(make_request()).await.unwrap();
    assert_eq!(second.status(), StatusCode::OK);
}

/// Regression (H1 IDOR): `store` must use the PATH ids, never the request body.
/// The body schema no longer carries ids at all, so stray `playlist_id`/
/// `episode_id` keys are ignored — posting to your OWN playlist with a body that
/// names a DIFFERENT playlist must not write into that other playlist.
#[tokio::test]
async fn test_store_uses_path_ids_not_body_ids() {
    use halogen_orm::episode_playlist::{Column, Entity as EpEntity};
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc.clone());

    let token = generate_jwt_token(payload["user_id"].as_str().unwrap());
    let pl1: i32 = payload["playlist_id_1"].as_str().unwrap().parse().unwrap();
    let pl2: i32 = payload["playlist_id_2"].as_str().unwrap().parse().unwrap();
    let ep1: i32 = payload["episode_id_1"].as_str().unwrap().parse().unwrap();
    let ep2: i32 = payload["episode_id_2"].as_str().unwrap().parse().unwrap();

    // PATH targets (pl1, ep2); the BODY tries to target (pl2, ep1).
    let body = serde_json::json!({ "data": { "playlist_id": pl2, "episode_id": ep1 } });
    let resp = router
        .oneshot(authed_json(
            "POST",
            format!("/api/v1/playlists/{pl1}/episodes/{ep2}"),
            &token,
            &body,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Path won: ep2 was added to pl1.
    let path_row = EpEntity::find()
        .filter(Column::PlaylistId.eq(pl1))
        .filter(Column::EpisodeId.eq(ep2))
        .one(&dbc)
        .await
        .unwrap();
    assert!(
        path_row.is_some(),
        "episode from the PATH must land in the PATH playlist"
    );

    // Body ignored: ep1 was NOT written into pl2 (the IDOR target).
    let body_row = EpEntity::find()
        .filter(Column::PlaylistId.eq(pl2))
        .filter(Column::EpisodeId.eq(ep1))
        .one(&dbc)
        .await
        .unwrap();
    assert!(
        body_row.is_none(),
        "body ids must be ignored — no write into the body's playlist"
    );

    let _ = dbc.close().await;
}

/// Removing an episode that is not a member of the playlist. `handle_delete`
/// looks the association up first and returns a not-found error (404) when it
/// is absent.
#[tokio::test]
async fn test_delete_episode_playlist_not_member_404() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    // episode_id_2 is seeded but not associated with playlist_id_1.
    let playlist_id: i32 = payload
        .get("playlist_id_1")
        .unwrap()
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let episode_id: i32 = payload
        .get("episode_id_2")
        .unwrap()
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let token = generate_jwt_token(user_id);

    let response = router
        .clone()
        .oneshot(authed(
            "DELETE",
            format!("/api/v1/playlists/{}/episodes/{}", playlist_id, episode_id),
            &token,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

/// `on_remove_delete_file_server` set: removing the episode's LAST playlist
/// membership deletes the staged server file and resets the episode to
/// `NotDownloaded`.
#[tokio::test]
async fn test_delete_with_server_flag_deletes_file() {
    use halogen_orm::episode::{
        ActiveModel as EpisodeActiveModel, Column as EpisodeColumn, Entity as EpisodeEntity,
    };
    use halogen_orm::playlist::ActiveModel as PlaylistActiveModel;
    use halogen_wire::DownloadStatus;
    use sea_orm::ActiveValue::Set;
    use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter};

    let (root, dbc, payload) = setup_test_db().await;
    let token = generate_jwt_token(payload["user_id"].as_str().unwrap());
    // episode_id_1 is a member of playlist_id_1 only.
    let playlist_id: i32 = payload["playlist_id_1"].as_str().unwrap().parse().unwrap();
    let episode_id: i32 = payload["episode_id_1"].as_str().unwrap().parse().unwrap();

    PlaylistActiveModel {
        id: Set(playlist_id),
        on_remove_delete_file_server: Set(true),
        ..Default::default()
    }
    .update(&dbc)
    .await
    .expect("set delete-on-remove flag");

    // Stage a real file and mark the episode downloaded.
    let file_path = root.path().join("ep.mp3");
    std::fs::write(&file_path, b"AUDIO").expect("write temp audio");
    EpisodeActiveModel {
        id: Set(episode_id),
        download_status: Set(DownloadStatus::Downloaded),
        content_file_path: Set(Some(file_path.to_string_lossy().to_string())),
        ..Default::default()
    }
    .update(&dbc)
    .await
    .expect("mark downloaded");

    let router = build_test_router(dbc.clone());
    let response = router
        .clone()
        .oneshot(authed(
            "DELETE",
            format!("/api/v1/playlists/{playlist_id}/episodes/{episode_id}"),
            &token,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert!(!file_path.exists(), "the server download must be deleted");
    let ep = EpisodeEntity::find()
        .filter(EpisodeColumn::Id.eq(episode_id))
        .one(&dbc)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(ep.download_status, DownloadStatus::NotDownloaded);
    assert!(ep.content_file_path.is_none(), "path cleared");
}

/// Flag unset (default): removal never touches the server download.
#[tokio::test]
async fn test_delete_without_server_flag_keeps_file() {
    use halogen_orm::episode::{
        ActiveModel as EpisodeActiveModel, Column as EpisodeColumn, Entity as EpisodeEntity,
    };
    use halogen_wire::DownloadStatus;
    use sea_orm::ActiveValue::Set;
    use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter};

    let (root, dbc, payload) = setup_test_db().await;
    let token = generate_jwt_token(payload["user_id"].as_str().unwrap());
    let playlist_id: i32 = payload["playlist_id_1"].as_str().unwrap().parse().unwrap();
    let episode_id: i32 = payload["episode_id_1"].as_str().unwrap().parse().unwrap();

    let file_path = root.path().join("ep.mp3");
    std::fs::write(&file_path, b"AUDIO").expect("write temp audio");
    EpisodeActiveModel {
        id: Set(episode_id),
        download_status: Set(DownloadStatus::Downloaded),
        content_file_path: Set(Some(file_path.to_string_lossy().to_string())),
        ..Default::default()
    }
    .update(&dbc)
    .await
    .expect("mark downloaded");

    let router = build_test_router(dbc.clone());
    let response = router
        .clone()
        .oneshot(authed(
            "DELETE",
            format!("/api/v1/playlists/{playlist_id}/episodes/{episode_id}"),
            &token,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert!(file_path.exists(), "flag off — the file must survive");
    let ep = EpisodeEntity::find()
        .filter(EpisodeColumn::Id.eq(episode_id))
        .one(&dbc)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(ep.download_status, DownloadStatus::Downloaded);
}

/// Flag set but the episode is still a member of ANOTHER playlist: the shared
/// server file survives; removing the last membership then deletes it.
#[tokio::test]
async fn test_delete_with_server_flag_spares_shared_episode() {
    use halogen_orm::episode::ActiveModel as EpisodeActiveModel;
    use halogen_orm::episode_playlist::ActiveModel as EpisodePlaylistActiveModel;
    use halogen_orm::playlist::ActiveModel as PlaylistActiveModel;
    use halogen_wire::DownloadStatus;
    use sea_orm::ActiveModelTrait;
    use sea_orm::ActiveValue::Set;

    let (root, dbc, payload) = setup_test_db().await;
    let token = generate_jwt_token(payload["user_id"].as_str().unwrap());
    let pl1: i32 = payload["playlist_id_1"].as_str().unwrap().parse().unwrap();
    let pl2: i32 = payload["playlist_id_2"].as_str().unwrap().parse().unwrap();
    let episode_id: i32 = payload["episode_id_1"].as_str().unwrap().parse().unwrap();

    // Both playlists flagged; the episode is a member of both.
    for pl in [pl1, pl2] {
        PlaylistActiveModel {
            id: Set(pl),
            on_remove_delete_file_server: Set(true),
            ..Default::default()
        }
        .update(&dbc)
        .await
        .expect("set delete-on-remove flag");
    }
    EpisodePlaylistActiveModel {
        episode_id: Set(episode_id),
        playlist_id: Set(pl2),
        position: Set(0),
        created_at: Set(chrono::Utc::now()),
        updated_at: Set(chrono::Utc::now()),
    }
    .insert(&dbc)
    .await
    .expect("add episode to second playlist");

    let file_path = root.path().join("ep.mp3");
    std::fs::write(&file_path, b"AUDIO").expect("write temp audio");
    EpisodeActiveModel {
        id: Set(episode_id),
        download_status: Set(DownloadStatus::Downloaded),
        content_file_path: Set(Some(file_path.to_string_lossy().to_string())),
        ..Default::default()
    }
    .update(&dbc)
    .await
    .expect("mark downloaded");

    let router = build_test_router(dbc.clone());

    // Remove from the first flagged playlist — still in pl2, file survives.
    let first = router
        .clone()
        .oneshot(authed(
            "DELETE",
            format!("/api/v1/playlists/{pl1}/episodes/{episode_id}"),
            &token,
        ))
        .await
        .unwrap();
    assert_eq!(first.status(), StatusCode::OK);
    assert!(
        file_path.exists(),
        "episode still in another playlist — the shared file must survive"
    );

    // Remove the last membership — now the file goes.
    let second = router
        .clone()
        .oneshot(authed(
            "DELETE",
            format!("/api/v1/playlists/{pl2}/episodes/{episode_id}"),
            &token,
        ))
        .await
        .unwrap();
    assert_eq!(second.status(), StatusCode::OK);
    assert!(
        !file_path.exists(),
        "last membership removed — file deleted"
    );
}

/// POST without a bearer token → the auth middleware rejects with 401.
#[tokio::test]
async fn test_store_episode_playlist_requires_auth() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let playlist_id = payload.get("playlist_id_2").unwrap().as_str().unwrap();
    let episode_id = payload.get("episode_id_2").unwrap().as_str().unwrap();

    let store_body = serde_json::json!({ "data": {} });

    let response = router
        .clone()
        .oneshot(json(
            "POST",
            format!("/api/v1/playlists/{}/episodes/{}", playlist_id, episode_id),
            &store_body,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

/// DELETE without a bearer token → the auth middleware rejects with 401.
#[tokio::test]
async fn test_delete_episode_playlist_requires_auth() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    // episode_id_1 is associated with playlist_id_1, but auth is missing so we
    // never reach the handler.
    let playlist_id = payload.get("playlist_id_1").unwrap().as_str().unwrap();
    let episode_id = payload.get("episode_id_1").unwrap().as_str().unwrap();

    let response = router
        .clone()
        .oneshot(unauthed(
            "DELETE",
            format!("/api/v1/playlists/{}/episodes/{}", playlist_id, episode_id),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

/// Reorder: add a second episode, move it to the front, and confirm the
/// playlist lists it first (positions rewritten 0..n).
#[tokio::test]
async fn test_move_episode_reorders_playlist() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(user_id);
    let playlist_id = payload.get("playlist_id_1").unwrap().as_str().unwrap();
    let episode_id_1 = payload.get("episode_id_1").unwrap().as_str().unwrap();
    let episode_id_2 = payload.get("episode_id_2").unwrap().as_str().unwrap();

    // Add episode 2 → playlist is now [ep1@0, ep2@1].
    // ids come from the path; body carries only the optional position.
    let add_body = serde_json::json!({ "data": {} });
    let add = router
        .clone()
        .oneshot(authed_json(
            "POST",
            format!(
                "/api/v1/playlists/{}/episodes/{}",
                playlist_id, episode_id_2
            ),
            &token,
            &add_body,
        ))
        .await
        .unwrap();
    assert_eq!(add.status(), StatusCode::OK);

    // Move episode 2 to index 0 → [ep2, ep1].
    let move_body = serde_json::json!({"data":{"to":0}});
    let mv = router
        .clone()
        .oneshot(authed_json(
            "POST",
            format!(
                "/api/v1/playlists/{}/episodes/{}/move",
                playlist_id, episode_id_2
            ),
            &token,
            &move_body,
        ))
        .await
        .unwrap();
    assert_eq!(mv.status(), StatusCode::OK);

    // Confirm order via the playlist episodes list. Queue (position) order is
    // opt-in via `order_by=position`; without it the list defaults to id order.
    let list = router
        .clone()
        .oneshot(authed(
            "GET",
            format!(
                "/api/v1/playlists/{}/episodes?order[order_by]=position&order[direction]=Asc",
                playlist_id
            ),
            &token,
        ))
        .await
        .unwrap();
    assert_eq!(list.status(), StatusCode::OK);
    let json = json_body(list).await;
    let data = json.get("data").unwrap().as_array().unwrap();
    let ids: Vec<String> = data
        .iter()
        .map(|e| e.get("id").unwrap().to_string())
        .collect();
    assert_eq!(ids.first().unwrap(), episode_id_2);
    assert_eq!(ids.get(1).unwrap(), episode_id_1);
}

/// Moving an episode that isn't a member of the playlist → 404.
#[tokio::test]
async fn test_move_episode_not_member_404() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let user_id = payload.get("user_id").unwrap().as_str().unwrap();
    let token = generate_jwt_token(user_id);
    // playlist_id_2 has no members.
    let playlist_id = payload.get("playlist_id_2").unwrap().as_str().unwrap();
    let episode_id = payload.get("episode_id_1").unwrap().as_str().unwrap();

    let move_body = serde_json::json!({"data":{"to":0}});
    let response = router
        .clone()
        .oneshot(authed_json(
            "POST",
            format!(
                "/api/v1/playlists/{}/episodes/{}/move",
                playlist_id, episode_id
            ),
            &token,
            &move_body,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

/// Move without a bearer token → 401.
#[tokio::test]
async fn test_move_episode_requires_auth() {
    let (_root, dbc, payload) = setup_test_db().await;
    let router = build_test_router(dbc);

    let playlist_id = payload.get("playlist_id_1").unwrap().as_str().unwrap();
    let episode_id = payload.get("episode_id_1").unwrap().as_str().unwrap();

    let move_body = serde_json::json!({"data":{"to":0}});
    let response = router
        .clone()
        .oneshot(json(
            "POST",
            format!(
                "/api/v1/playlists/{}/episodes/{}/move",
                playlist_id, episode_id
            ),
            &move_body,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}
