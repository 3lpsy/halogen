//! Bulk add/remove of many episodes to/from ONE playlist in a single request.
//!
//! These wrap the single-episode membership services (`handle_store` /
//! `handle_delete`) and loop — same services, same idempotency, just a list body
//! (`EpisodePlaylistBulkData`). Mirrors the episodes download bulk
//! ([`crate::routers::episodes::bulk`]).
//!
//! Authorization has two layers: the playlist is owner-guarded ONCE (the whole
//! request targets a single playlist), and episode ids are authorized **per id,
//! before the loop** — ids the actor isn't subscribed to (and doesn't own / isn't
//! admin for) are silently dropped. Per-id work is also lenient: a failure (e.g.
//! removing an episode that isn't a member) is logged and skipped so the rest still
//! apply. So a single bad id never fails the batch (noop on auth failure / on an
//! already-present add / on a not-a-member remove).

use axum::{Extension, Json, http::StatusCode, response::IntoResponse};
use halogen_wire::{EpisodePlaylistBulkData, ResponseData};
use sea_orm::DatabaseConnection;
use tracing::warn;

use crate::handlers::playlist::episode_playlist::{
    handle_delete as episode_playlist_delete, handle_store as episode_playlist_store,
    server_delete_flag,
};
use crate::routers::ApiError;
use crate::routers::extractors::{Actor, Body, Id};
use crate::routers::guards::{self, require_episode_subscribed};

/// Keep only the ids the actor may act on (subscribed / owner / admin), preserving
/// order. One guard lookup per id — fine for the bounded (`max = 500`) list.
async fn authorized_ids(dbc: &DatabaseConnection, actor: Actor, ids: &[i32]) -> Vec<i32> {
    let mut kept = Vec::with_capacity(ids.len());
    for &episode_id in ids {
        if require_episode_subscribed(dbc, actor, episode_id)
            .await
            .is_ok()
        {
            kept.push(episode_id);
        }
    }
    kept
}

/// POST /playlists/{id}/episodes/bulk — add each authorized episode id to the
/// playlist (append; idempotent on re-add). Lenient: a per-id failure is logged and
/// skipped so the rest still apply. **200 OK** with the single route's empty
/// `()` envelope.
pub async fn store_bulk(
    Extension(dbc): Extension<DatabaseConnection>,
    actor: Actor,
    Id(playlist_id): Id,
    Body(data): Body<EpisodePlaylistBulkData>,
) -> Result<impl IntoResponse, ApiError> {
    // Mutating a playlist's membership requires owning the playlist (or admin).
    guards::require_playlist_owner_or_admin(&dbc, actor, playlist_id).await?;
    let ids = authorized_ids(&dbc, actor, &data.episode_ids).await;
    for episode_id in ids {
        // Append (`None`) — the single-add front-of-queue nicety isn't applied in bulk.
        if let Err(e) = episode_playlist_store(&dbc, playlist_id, episode_id, None).await {
            warn!(playlist_id, episode_id, error = ?e, "bulk add to playlist failed");
        }
    }
    Ok((StatusCode::OK, Json(ResponseData::from_data(()))))
}

/// DELETE /playlists/{id}/episodes/bulk — remove each authorized episode id from the
/// playlist. Lenient: removing a non-member (or any per-id error) is logged and
/// skipped so the rest still apply. **200 OK**.
pub async fn delete_bulk(
    Extension(dbc): Extension<DatabaseConnection>,
    actor: Actor,
    Id(playlist_id): Id,
    Body(data): Body<EpisodePlaylistBulkData>,
) -> Result<impl IntoResponse, ApiError> {
    guards::require_playlist_owner_or_admin(&dbc, actor, playlist_id).await?;
    // Per-playlist cleanup flag, fetched once for the whole batch (the request
    // targets a single playlist).
    let delete_server_file = server_delete_flag(&dbc, playlist_id).await?;
    let ids = authorized_ids(&dbc, actor, &data.episode_ids).await;
    for episode_id in ids {
        if let Err(e) =
            episode_playlist_delete(&dbc, episode_id, playlist_id, delete_server_file).await
        {
            warn!(playlist_id, episode_id, error = ?e, "bulk remove from playlist failed");
        }
    }
    Ok((StatusCode::OK, Json(ResponseData::from_data(()))))
}

#[cfg(test)]
mod tests {
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
}
