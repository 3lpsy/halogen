use axum::{Extension, Json};
use halogen_wire::{PodcastConfigData, PodcastConfigUpdateData, ResponseData};
use sea_orm::DatabaseConnection;
use tracing::warn;

use crate::handlers::podcast_config::podcast_config_update;
use crate::routers::errors::ApiError;
use crate::routers::extractors::{Actor, Body, Id};
use crate::routers::guards;

pub async fn update(
    Extension(dbc): Extension<DatabaseConnection>,
    actor: Actor,
    Id(config_id): Id,
    Body(payload): Body<PodcastConfigUpdateData>,
) -> Result<Json<ResponseData<PodcastConfigData>>, ApiError> {
    // Owner-or-admin only. The DTO was validated by the `Body` extractor; the
    // guard runs after body extraction and before any mutation.
    guards::require_config_owner_or_admin(&dbc, actor, config_id).await?;

    match podcast_config_update::handle(&dbc, config_id, payload).await {
        Ok(data) => Ok(Json(ResponseData::from_data(data))),
        Err(err) => {
            warn!("Error updating podcast config: {:?}", err);
            Err(ApiError::from(err))
        }
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
