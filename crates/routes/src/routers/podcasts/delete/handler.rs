use axum::{Extension, Json};
use halogen_wire::ResponseData;
use sea_orm::DatabaseConnection;
use tracing::warn;

use crate::handlers::podcast::podcast_delete;
use crate::routers::ApiError;
use crate::routers::extractors::{Actor, Id};
use crate::routers::guards;

pub async fn delete(
    Extension(dbc): Extension<DatabaseConnection>,
    actor: Actor,
    Id(podcast_id): Id,
) -> Result<Json<ResponseData<()>>, ApiError> {
    guards::require_podcast_owner_or_admin(&dbc, actor, podcast_id).await?;

    // The handler confirms existence (404 if gone) and cascades the delete; it owns
    // the title for logging, so the router no longer pre-fetches the podcast.
    podcast_delete::handle(&dbc, podcast_id)
        .await
        .map_err(|err| {
            warn!("Error deleting podcast: {:?}", err);
            ApiError(err)
        })?;

    Ok(Json(ResponseData::from_data(())))
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
