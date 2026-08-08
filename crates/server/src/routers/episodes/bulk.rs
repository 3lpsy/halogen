//! Bulk download/remove over many episodes in one request.
//!
//! These mirror the single-episode `download`/`remove` handlers exactly — same
//! services, same per-episode authorization — but take a list of ids in the body
//! (`EpisodeBulkActionData`) and loop. The only real difference is the API
//! contract; the actual work is the shared per-episode code.
//!
//! Authorization is applied **per id, before the loop**: ids the actor isn't
//! subscribed to (and doesn't own / isn't admin for) are silently dropped, so a
//! bulk request does the operations the actor is allowed to do and skips the rest
//! (no whole-request failure for a single unauthorized id).

use axum::{Extension, Json, http::StatusCode, response::IntoResponse};
use halogen_wire::{EpisodeBulkActionData, ResponseData};
use sea_orm::DatabaseConnection;

use super::download::{MediaDownloadConfig, spawn_episode_download};
use crate::routers::ApiError;
use crate::routers::extractors::{Actor, Body};
use crate::routers::guards::require_episode_subscribed;
use halogen_download::remove_server_download;

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

/// POST /episodes/download/bulk — trigger a server-side fetch for each authorized
/// episode id. Returns **202 Accepted** immediately (fetches run in the background,
/// like the single route). Mirrors the single route's empty `()` envelope.
pub async fn download_bulk(
    Extension(dbc): Extension<DatabaseConnection>,
    Extension(cfg): Extension<MediaDownloadConfig>,
    actor: Actor,
    Body(data): Body<EpisodeBulkActionData>,
) -> Result<impl IntoResponse, ApiError> {
    let ids = authorized_ids(&dbc, actor, &data.episode_ids).await;
    for episode_id in ids {
        spawn_episode_download(dbc.clone(), cfg.clone(), episode_id);
    }
    Ok((StatusCode::ACCEPTED, Json(ResponseData::from_data(()))))
}

/// DELETE /episodes/download/bulk — remove the server's downloaded copy for each
/// authorized episode id. Runs inline (each removal is fast); lenient — a removal
/// that errors is logged and skipped so the rest still apply. **200 OK**.
pub async fn remove_bulk(
    Extension(dbc): Extension<DatabaseConnection>,
    actor: Actor,
    Body(data): Body<EpisodeBulkActionData>,
) -> Result<impl IntoResponse, ApiError> {
    let ids = authorized_ids(&dbc, actor, &data.episode_ids).await;
    for episode_id in ids {
        if let Err(e) = remove_server_download(&dbc, episode_id).await {
            tracing::warn!(episode_id, error = %e, "bulk remove server download failed");
        }
    }
    Ok((StatusCode::OK, Json(ResponseData::from_data(()))))
}

#[cfg(test)]
mod tests {
    use crate::routers::episodes::tests::{build_test_router, generate_jwt_token, setup_test_db};
    use crate::tests::harness::authed_json;
    use axum::http::StatusCode;
    use halogen_orm::{episode, podcast};
    use halogen_wire::DownloadStatus;
    use sea_orm::{ActiveModelTrait, EntityTrait};
    use tower::ServiceExt;

    fn bulk_body(ids: &[i32]) -> serde_json::Value {
        serde_json::json!({ "data": { "episode_ids": ids } })
    }

    /// Mark an episode downloaded with a real on-disk file (so `remove` has
    /// something to delete + can flip the status back).
    async fn mark_downloaded(dbc: &sea_orm::DatabaseConnection, id: i32, path: &std::path::Path) {
        std::fs::write(path, b"AUDIO").expect("write temp audio");
        episode::ActiveModel {
            id: sea_orm::ActiveValue::set(id),
            download_status: sea_orm::ActiveValue::set(DownloadStatus::Downloaded),
            content_file_path: sea_orm::ActiveValue::set(Some(path.to_string_lossy().to_string())),
            ..Default::default()
        }
        .update(dbc)
        .await
        .expect("mark downloaded");
    }

    // POST /episodes/download/bulk with two owned ids → 202 (background fetch).
    #[tokio::test]
    async fn test_download_bulk_owned_ids_returns_accepted() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(user_id);
        let ep1: i32 = payload
            .get("episode_id_1")
            .unwrap()
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        let ep2: i32 = payload
            .get("episode_id_2")
            .unwrap()
            .as_str()
            .unwrap()
            .parse()
            .unwrap();

        let response = router
            .oneshot(authed_json(
                "POST",
                "/api/v1/episodes/download/bulk",
                &token,
                &bulk_body(&[ep1, ep2]),
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::ACCEPTED);
    }

    // An unauthorized id (a podcast the actor isn't subscribed to / doesn't own) is
    // filtered out, not failed. Verified deterministically through DELETE (which is
    // synchronous): the authorized episode is removed, the unauthorized one is left
    // untouched.
    #[tokio::test]
    async fn test_remove_bulk_filters_unauthorized_ids() {
        let (root, dbc, payload) = setup_test_db().await;

        // A second podcast owned by the admin (not the regular user) + an episode in
        // it — the regular user is neither owner nor subscriber, so it's filtered.
        let admin_id: i32 = payload
            .get("admin_id")
            .unwrap()
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        podcast::ActiveModel {
            id: sea_orm::ActiveValue::set(1000),
            title: sea_orm::ActiveValue::set("Other".to_string()),
            description: sea_orm::ActiveValue::set("d".to_string()),
            feed_url: sea_orm::ActiveValue::set("https://example.com/other.xml".to_string()),
            art_url: sea_orm::ActiveValue::set(None),
            art_file_path: sea_orm::ActiveValue::set(None),
            author: sea_orm::ActiveValue::set(None),
            etag: sea_orm::ActiveValue::set(None),
            last_modified: sea_orm::ActiveValue::set(None),
            feed_url_redirects: sea_orm::ActiveValue::set(None),
            polled_at: sea_orm::ActiveValue::set(None),
            podcast_config_id: sea_orm::ActiveValue::set(None),
            owner_id: sea_orm::ActiveValue::set(admin_id),
            created_at: sea_orm::ActiveValue::set(chrono::Utc::now()),
            updated_at: sea_orm::ActiveValue::set(chrono::Utc::now()),
        }
        .insert(&dbc)
        .await
        .unwrap();
        episode::ActiveModel {
            id: sea_orm::ActiveValue::set(1001),
            podcast_id: sea_orm::ActiveValue::set(1000),
            title: sea_orm::ActiveValue::set("Other Ep".to_string()),
            description: sea_orm::ActiveValue::set("d".to_string()),
            content_url: sea_orm::ActiveValue::set("https://example.com/o.mp3".to_string()),
            guid: sea_orm::ActiveValue::set(None),
            art_url: sea_orm::ActiveValue::set(None),
            published_at: sea_orm::ActiveValue::set(Some(chrono::Utc::now())),
            downloaded_at: sea_orm::ActiveValue::set(None),
            content_file_path: sea_orm::ActiveValue::set(None),
            download_size: sea_orm::ActiveValue::set(None),
            art_file_path: sea_orm::ActiveValue::set(None),
            download_status: sea_orm::ActiveValue::set(DownloadStatus::NotDownloaded),
            download_started_at: sea_orm::ActiveValue::set(None),
            download_attempts: sea_orm::ActiveValue::set(0),
            duration_secs: sea_orm::ActiveValue::set(None),
            created_at: sea_orm::ActiveValue::set(chrono::Utc::now()),
            updated_at: sea_orm::ActiveValue::set(chrono::Utc::now()),
        }
        .insert(&dbc)
        .await
        .unwrap();

        let ep1: i32 = payload
            .get("episode_id_1")
            .unwrap()
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        let authorized_file = root.path().join("authorized.mp3");
        let other_file = root.path().join("other.mp3");
        mark_downloaded(&dbc, ep1, &authorized_file).await;
        mark_downloaded(&dbc, 1001, &other_file).await;

        let router = build_test_router(dbc.clone());
        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(user_id);

        let response = router
            .oneshot(authed_json(
                "DELETE",
                "/api/v1/episodes/download/bulk",
                &token,
                &bulk_body(&[ep1, 1001]),
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        // Authorized episode: removed (file gone, status reset).
        let authorized = episode::Entity::find_by_id(ep1)
            .one(&dbc)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(authorized.download_status, DownloadStatus::NotDownloaded);
        assert!(!authorized_file.exists(), "authorized file deleted");

        // Unauthorized episode: untouched (filtered out before the loop).
        let other = episode::Entity::find_by_id(1001)
            .one(&dbc)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(other.download_status, DownloadStatus::Downloaded);
        assert!(other_file.exists(), "unauthorized file preserved");
    }

    // An empty id list fails the DTO validation in the `Body` extractor → 400.
    #[tokio::test]
    async fn test_download_bulk_empty_is_400() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);
        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(user_id);

        let response = router
            .oneshot(authed_json(
                "POST",
                "/api/v1/episodes/download/bulk",
                &token,
                &bulk_body(&[]),
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}
