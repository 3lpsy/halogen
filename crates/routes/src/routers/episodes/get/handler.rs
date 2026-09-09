use axum::{Extension, Json};
use halogen_wire::{EpisodeData, EpisodeShowParams, ResponseData};
use sea_orm::DatabaseConnection;

use crate::handlers::episode::episode_get;
use crate::routers::ApiError;
use crate::routers::extractors::{Actor, Id, Query};
use crate::routers::guards;

/// Fetch a single episode by ID. Read-gated on being subscribed to the episode's podcast (or owning it /
/// admin), the same access the list route grants — a non-subscriber gets 404 so existence doesn't leak.
/// `playback_status` is per-user, so the caller's id is also threaded into the handler.
pub async fn get(
    Extension(dbc): Extension<DatabaseConnection>,
    actor: Actor,
    id: Id,
    Query(params): Query<EpisodeShowParams>,
) -> Result<Json<ResponseData<EpisodeData>>, ApiError> {
    guards::require_episode_subscribed(&dbc, actor, id.0).await?;
    let episode_data = episode_get::handle(&dbc, actor.id, id.0, params.includes.as_ref())
        .await
        .map_err(ApiError)?;
    Ok(Json(ResponseData::from_data(episode_data)))
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
