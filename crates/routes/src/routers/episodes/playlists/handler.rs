use axum::{Extension, Json};
use halogen_wire::{DefaultListParams, PlaylistData, PlaylistInclude, ResponseData};
use sea_orm::DatabaseConnection;
use tracing::warn;

use crate::handlers::episode::episode_playlists::handle as episode_playlists;
use crate::routers::ApiError;
use crate::routers::extractors::{Actor, Id, Query};
use crate::routers::guards;

/// `GET /episodes/{id}/playlists` — the caller's playlists that contain this
/// episode (pre-selection for the "add to playlist" picker).
pub async fn list(
    Extension(dbc): Extension<DatabaseConnection>,
    actor: Actor,
    Id(episode_id): Id,
    Query(params): Query<DefaultListParams<PlaylistInclude>>,
) -> Result<Json<ResponseData<Vec<PlaylistData>>>, ApiError> {
    // Read-gate on subscription like the other episode-id routes (404 hides existence).
    guards::require_episode_subscribed(&dbc, actor, episode_id).await?;
    let (data, paginator) = episode_playlists(&dbc, actor.id, episode_id, &params)
        .await
        .map_err(|err| {
            warn!("Error fetching episode playlists: {:?}", err);
            err
        })?;

    Ok(Json(ResponseData::from_paginator(data, paginator)))
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
