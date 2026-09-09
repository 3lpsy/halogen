use axum::{Extension, Json};
use halogen_wire::{EpisodeData, EpisodeUpdateData, ResponseData};
use sea_orm::DatabaseConnection;

use crate::handlers::episode::episode_update;
use crate::routers::ApiError;
use crate::routers::extractors::{Actor, Body, Id};
use crate::routers::guards;

pub async fn update(
    Extension(dbc): Extension<DatabaseConnection>,
    actor: Actor,
    id: Id,
    Body(update_data): Body<EpisodeUpdateData>,
) -> Result<Json<ResponseData<EpisodeData>>, ApiError> {
    // Write gate: only the episode's podcast owner (or an admin) may edit it.
    guards::require_episode_writer(&dbc, actor, id.0).await?;

    let ep_data = episode_update::handle(&dbc, actor.id, id.0, &update_data)
        .await
        .map_err(ApiError)?;
    Ok(Json(ResponseData::from_data(ep_data)))
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
