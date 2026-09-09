use axum::{Extension, Json};
use halogen_wire::{PodcastData, PodcastUpdateData, ResponseData};
use sea_orm::DatabaseConnection;
use tracing::warn;

use crate::handlers::podcast::podcast_update;
use crate::routers::ApiError;
use crate::routers::extractors::{Actor, Body, Id};
use crate::routers::guards;

pub async fn update(
    Extension(dbc): Extension<DatabaseConnection>,
    actor: Actor,
    Id(podcast_id): Id,
    Body(update_data): Body<PodcastUpdateData>,
) -> Result<Json<ResponseData<PodcastData>>, ApiError> {
    // After body validation, before mutating: owner-or-admin only.
    guards::require_podcast_owner_or_admin(&dbc, actor, podcast_id).await?;
    let podcast_data = podcast_update::handle(&dbc, podcast_id, update_data)
        .await
        .map_err(|err| {
            warn!("Error updating podcast: {:?}", err);
            err
        })?;

    Ok(Json(ResponseData::from_data(podcast_data)))
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
