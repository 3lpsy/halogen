use axum::{Extension, Json};
use halogen_wire::{DefaultGetParams, PodcastData, PodcastInclude, ResponseData};
use sea_orm::DatabaseConnection;
use tracing::warn;

use crate::handlers::podcast::podcast_get;
use crate::routers::ApiError;
use crate::routers::extractors::{Actor, Id, Query};
use crate::routers::guards;

pub async fn get(
    Extension(dbc): Extension<DatabaseConnection>,
    actor: Actor,
    Id(podcast_id): Id,
    Query(params): Query<DefaultGetParams<PodcastInclude>>,
) -> Result<Json<ResponseData<PodcastData>>, ApiError> {
    // Read gate: only subscribers (or owner/admin) may see a podcast; 404 otherwise.
    guards::require_subscribed(&dbc, actor, podcast_id).await?;
    let podcast_data = podcast_get::handle(&dbc, podcast_id, params.includes.as_ref())
        .await
        .map_err(|err| {
            warn!("Error fetching podcast: {:?}", err);
            err
        })?;

    Ok(Json(ResponseData::from_data(podcast_data)))
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
