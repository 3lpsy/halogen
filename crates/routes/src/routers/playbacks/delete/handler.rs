use crate::routers::extractors::{AuthUserId, Id};
use axum::{Extension, Json};
use halogen_wire::ResponseData;
use sea_orm::DatabaseConnection;
use tracing::warn;

use crate::handlers::playback::playback_delete::handle as playback_delete;
use crate::routers::ApiError;

pub async fn delete(
    Extension(dbc): Extension<DatabaseConnection>,
    AuthUserId(user_id): AuthUserId,
    Id(playback_id): Id,
) -> Result<Json<ResponseData<()>>, ApiError> {
    playback_delete(&dbc, user_id, playback_id)
        .await
        .map_err(|err| {
            warn!("Error deleting playback: {:?}", err);
            err
        })?;

    Ok(Json(ResponseData::from_data(())))
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
