use std::path::PathBuf;
use std::sync::Arc;

use axum::{Extension, Json, http::StatusCode, response::IntoResponse};
use halogen_wire::ResponseData;
use sea_orm::DatabaseConnection;

use crate::routers::ApiError;
use crate::routers::extractors::{Actor, Id};
use crate::routers::guards::require_episode_subscribed;
use halogen_download::{
    DownloadOptions, DownloadTracker, RetryPolicy, download_episode, enforce_retention_for_episode,
    remove_server_download,
};

/// Per-deployment knobs the on-demand download handler needs, layered as an
/// `Extension` in `build_router` (mirrors the resolved `Config`).
#[derive(Clone)]
pub struct MediaDownloadConfig {
    pub media_root: PathBuf,
    pub use_mock_download: bool,
    /// Global fallback retention cap, used when the episode's podcast has no
    /// per-podcast `max_episodes`.
    pub fallback_max_episodes: usize,
    /// Shared in-flight download progress tracker (the same `Arc` the polling
    /// handle owns and the progress API reads).
    pub tracker: Arc<DownloadTracker>,
}

/// Spawn the background fetch of one episode's audio into `media_root`, then
/// enforce its podcast's retention cap. Fire-and-forget: errors are logged, never
/// returned (the route has already acknowledged with 202). Shared by the single
/// `download` handler and the bulk `download_bulk` handler so they fetch identically.
pub(super) fn spawn_episode_download(
    dbc: DatabaseConnection,
    cfg: MediaDownloadConfig,
    episode_id: i32,
) {
    tokio::spawn(async move {
        let opts = DownloadOptions {
            media_root: cfg.media_root.clone(),
            use_mock_download: cfg.use_mock_download,
            tracker: cfg.tracker.clone(),
            retry: RetryPolicy::production(),
        };
        if let Err(e) = download_episode(&dbc, episode_id, &opts).await {
            tracing::warn!(episode_id, error = %e, "on-demand episode download failed");
            return;
        }
        // A new server download may push the podcast over its retention cap —
        // purge the oldest (resolved per-podcast, else the global fallback).
        if let Err(e) =
            enforce_retention_for_episode(&dbc, episode_id, cfg.fallback_max_episodes).await
        {
            tracing::warn!(episode_id, error = %e, "retention after download failed");
        }
    });
}

/// POST /episodes/{id}/download — trigger a server-side fetch of the episode's
/// audio into `media_root`.
///
/// Authorization: the actor must be subscribed to (or own / admin) the episode's
/// podcast — same access the read endpoints require.
///
/// Idempotent: the download service skips `Downloading`/`Downloaded` and retries
/// `DownloadError`. The fetch runs in the background and we return **202 Accepted**
/// immediately; the client observes `download_status` flip on its next sync pull
/// (and can then stream it from `GET /episodes/{id}/audio`).
pub async fn download(
    Extension(dbc): Extension<DatabaseConnection>,
    Extension(cfg): Extension<MediaDownloadConfig>,
    actor: Actor,
    id: Id,
) -> Result<impl IntoResponse, ApiError> {
    require_episode_subscribed(&dbc, actor, id.0).await?;
    spawn_episode_download(dbc, cfg, id.0);
    Ok((StatusCode::ACCEPTED, Json(ResponseData::from_data(()))))
}

/// DELETE /episodes/{id}/download — remove the server's downloaded copy: delete
/// the file and reset the episode to `NotDownloaded`. Runs inline (fast) and
/// returns **200 OK**; the client sees the status flip on its next sync pull.
///
/// Authorization: same subscription gate as `download`.
pub async fn remove(
    Extension(dbc): Extension<DatabaseConnection>,
    actor: Actor,
    id: Id,
) -> Result<impl IntoResponse, ApiError> {
    require_episode_subscribed(&dbc, actor, id.0).await?;
    match remove_server_download(&dbc, id.0).await {
        Ok(()) => Ok((StatusCode::OK, Json(ResponseData::from_data(())))),
        Err(e) => {
            tracing::warn!(episode_id = id.0, error = %e, "remove server download failed");
            // Return the standard error envelope (not a bare status) so clients
            // get the same `ResponseData` shape as every other endpoint.
            Err(ApiError::new(
                halogen_utils::constants::VALIDATION_DATABASE_FIELD,
                halogen_utils::constants::VALIDATION_PANIC_CODE,
                "Failed to remove download".to_string(),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
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
}
