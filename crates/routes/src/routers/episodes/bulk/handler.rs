//! Bulk episode download/remove reuses single-episode services. Filter every ID by subscription/ownership/admin access
//! before processing; unauthorized IDs are skipped without failing the whole batch.

use axum::{Extension, Json, http::StatusCode, response::IntoResponse};
use halogen_wire::{EpisodeBulkActionData, ResponseData};
use sea_orm::DatabaseConnection;

use super::super::download::{MediaDownloadConfig, spawn_episode_download};
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
#[path = "tests.rs"]
mod tests;
