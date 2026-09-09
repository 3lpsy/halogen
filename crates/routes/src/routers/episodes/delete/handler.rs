use axum::{Extension, Json};
use halogen_wire::{EpisodeDeleteParams, ResponseData};
use sea_orm::DatabaseConnection;
use tracing::warn;

use crate::handlers::episode::episode_delete;
use crate::routers::ApiError;
use crate::routers::extractors::{Actor, Id};
use crate::routers::guards;

/// Delete an episode by ID.
pub async fn delete(
    Extension(dbc): Extension<DatabaseConnection>,
    actor: Actor,
    id: Id,
) -> Result<Json<ResponseData<()>>, ApiError> {
    let episode_id = id.0;

    // Write gate: only the episode's podcast owner (or an admin) may delete it.
    // Also yields the 404 (keyed "id") for a missing episode, replacing the
    // old existence probe.
    guards::require_episode_writer(&dbc, actor, episode_id).await?;

    let delete_params = EpisodeDeleteParams { id: episode_id };
    episode_delete::handle(&dbc, &delete_params)
        .await
        .map_err(|err| {
            warn!("Error deleting episode: {:?}", err);
            ApiError(err)
        })?;

    Ok(Json(ResponseData::from_data(())))
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
