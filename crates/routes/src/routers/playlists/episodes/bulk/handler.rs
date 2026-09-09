//! Bulk membership changes authorize the target playlist once, then filter episode IDs by subscription/ownership/admin
//! access. Reuse idempotent single-item services; log and skip per-ID failures so unauthorized, present, or missing
//! memberships cannot fail the batch.

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
#[path = "tests.rs"]
mod tests;
