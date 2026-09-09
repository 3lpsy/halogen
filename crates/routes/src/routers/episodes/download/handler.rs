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
pub(in crate::routers::episodes) fn spawn_episode_download(
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

/// Authorize subscription/ownership/admin access, then start an episode download and return 202. The service skips
/// Downloading/Downloaded and retries DownloadError; clients observe durable status through sync and can stream when
/// complete.
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

/// DELETE /episodes/{id}/download — remove the server's downloaded copy: delete the file and reset the episode
/// to `NotDownloaded`. Runs inline (fast) and returns **200 OK**; the client sees the status flip on its next
/// sync pull. Authorization: same subscription gate as `download`.
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
#[path = "tests.rs"]
mod tests;
