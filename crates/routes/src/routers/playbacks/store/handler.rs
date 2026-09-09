use axum::{Extension, Json};
use halogen_wire::{PlaybackData, PlaybackStoreData, ResponseData};
use sea_orm::DatabaseConnection;
use tracing::warn;

use crate::handlers::playback::PlaybackCompleteConfig;
use crate::handlers::playback::playback_store::handle as playback_store;
use crate::routers::errors::ApiError;
use crate::routers::extractors::{AuthUserId, Body};

pub async fn store(
    Extension(dbc): Extension<DatabaseConnection>,
    Extension(PlaybackCompleteConfig(complete_pct)): Extension<PlaybackCompleteConfig>,
    AuthUserId(user_id): AuthUserId,
    Body(store_data): Body<PlaybackStoreData>,
) -> Result<Json<ResponseData<PlaybackData>>, ApiError> {
    let playback_data = playback_store(&dbc, user_id, store_data, complete_pct)
        .await
        .map_err(|err| {
            warn!("Error creating playback: {:?}", err);
            ApiError::from(err)
        })?;

    Ok(Json(ResponseData::from_data(playback_data)))
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
