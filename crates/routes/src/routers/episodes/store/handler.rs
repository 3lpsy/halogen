use axum::{Extension, Json};
use halogen_wire::{EpisodeData, EpisodeStoreData, ResponseData};
use sea_orm::DatabaseConnection;

use crate::handlers::episode::episode_store;
use crate::routers::ApiError;
use crate::routers::extractors::{Actor, Body};
use crate::routers::guards;

pub async fn store(
    Extension(dbc): Extension<DatabaseConnection>,
    actor: Actor,
    Body(store_data): Body<EpisodeStoreData>,
) -> Result<Json<ResponseData<EpisodeData>>, ApiError> {
    // Create gate: only the parent podcast's owner (or an admin) may add episodes.
    guards::require_podcast_owner_or_admin(&dbc, actor, store_data.podcast_id).await?;

    let ep_data = episode_store::handle(&dbc, store_data)
        .await
        .map_err(ApiError)?;
    Ok(Json(ResponseData::from_data(ep_data)))
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
