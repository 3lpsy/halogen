//! Nested POST/DELETE podcast config routes atomically create/link or unlink/delete config rows. Existing configs use
//! standalone GET/PUT; no standalone create/delete exists, preserving podcast_config_id consistency.

use axum::{Extension, Json};
use halogen_wire::{PodcastConfigData, PodcastConfigStoreData, ResponseData};
use sea_orm::DatabaseConnection;

use crate::handlers::podcast_config::{remove_for_podcast, store_for_podcast};
use crate::routers::ApiError;
use crate::routers::extractors::{Actor, Body, Id};
use crate::routers::guards;

/// `POST /podcasts/{id}/config` — create + link a config for the podcast.
pub async fn store(
    Extension(dbc): Extension<DatabaseConnection>,
    actor: Actor,
    Id(podcast_id): Id,
    Body(data): Body<PodcastConfigStoreData>,
) -> Result<Json<ResponseData<PodcastConfigData>>, ApiError> {
    // Only the podcast's owner (or an admin) may set its config.
    guards::require_podcast_config_writer(&dbc, actor, podcast_id).await?;
    let config = store_for_podcast::handle(&dbc, podcast_id, data).await?;
    Ok(Json(ResponseData::from_data(config)))
}

/// `DELETE /podcasts/{id}/config` — unlink + delete the podcast's config.
pub async fn delete(
    Extension(dbc): Extension<DatabaseConnection>,
    actor: Actor,
    Id(podcast_id): Id,
) -> Result<Json<ResponseData<()>>, ApiError> {
    guards::require_podcast_config_writer(&dbc, actor, podcast_id).await?;
    remove_for_podcast::handle(&dbc, podcast_id).await?;
    Ok(Json(ResponseData::from_data(())))
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
