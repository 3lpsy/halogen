use crate::routers::extractors::{AuthUserId, Id};
use axum::{Extension, Json};
use halogen_wire::{PlaybackData, ResponseData};
use sea_orm::DatabaseConnection;
use tracing::warn;

use crate::handlers::playback::playback_get::handle as playback_get;
use crate::routers::ApiError;

pub async fn get(
    Extension(dbc): Extension<DatabaseConnection>,
    AuthUserId(user_id): AuthUserId,
    Id(playback_id): Id,
) -> Result<Json<ResponseData<PlaybackData>>, ApiError> {
    let playback_data = playback_get(&dbc, user_id, playback_id)
        .await
        .map_err(|err| {
            warn!("Error fetching playback: {:?}", err);
            err
        })?;

    Ok(Json(ResponseData::from_data(playback_data)))
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
