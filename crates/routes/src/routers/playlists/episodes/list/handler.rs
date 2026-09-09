use axum::{Extension, Json};
use halogen_wire::{DefaultListParams, EpisodeData, EpisodeInclude, ResponseData};
use sea_orm::DatabaseConnection;
use tracing::warn;

use crate::handlers::playlist::playlist_episodes;
use crate::routers::ApiError;
use crate::routers::extractors::{Actor, Id, Query};
use crate::routers::guards;

/// GET /playlists/{id}/episodes — list episodes belonging to a playlist.
pub async fn list(
    Extension(dbc): Extension<DatabaseConnection>,
    actor: Actor,
    Id(playlist_id): Id,
    Query(params): Query<DefaultListParams<EpisodeInclude>>,
) -> Result<Json<ResponseData<Vec<EpisodeData>>>, ApiError> {
    // Reading a playlist's episodes is gated on owning the playlist (or admin);
    // 404 for a non-owned/missing playlist so existence doesn't leak.
    guards::require_playlist_owner_or_admin(&dbc, actor, playlist_id).await?;
    let (episodes, paginator) = playlist_episodes::handle(&dbc, actor.id, playlist_id, &params)
        .await
        .map_err(|err| {
            warn!("Error fetching playlist episodes: {:?}", err);
            err
        })?;

    Ok(Json(ResponseData::from_paginator(episodes, paginator)))
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
