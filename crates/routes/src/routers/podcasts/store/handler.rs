use axum::{Extension, Json};
use halogen_wire::{PodcastData, PodcastStoreData, ResponseData};
use sea_orm::DatabaseConnection;
use tracing::warn;

use crate::handlers::podcast::podcast_store;
use crate::routers::ApiError;
use crate::routers::extractors::{AuthUserId, Body};

pub async fn store(
    Extension(dbc): Extension<DatabaseConnection>,
    AuthUserId(owner_id): AuthUserId,
    Body(store_data): Body<PodcastStoreData>,
) -> Result<Json<ResponseData<PodcastData>>, ApiError> {
    let podcast_data = podcast_store::handle(&dbc, owner_id, store_data)
        .await
        .map_err(|err| {
            warn!("Error creating podcast: {:?}", err);
            err
        })?;

    Ok(Json(ResponseData::from_data(podcast_data)))
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
