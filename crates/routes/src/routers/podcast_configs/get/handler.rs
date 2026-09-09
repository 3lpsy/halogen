use axum::{Extension, Json};
use halogen_wire::{PodcastConfigData, ResponseData};
use sea_orm::DatabaseConnection;

use crate::handlers::podcast_config::podcast_config_get;
use crate::routers::ApiError;
use crate::routers::extractors::{Actor, Id};
use crate::routers::guards;

pub async fn get(
    Extension(dbc): Extension<DatabaseConnection>,
    actor: Actor,
    Id(config_id): Id,
) -> Result<Json<ResponseData<PodcastConfigData>>, ApiError> {
    // Read gate: only an owner of the podcast that references this config (or an
    // admin) may read it. A standalone config is owner-undecidable → 403 for a
    // non-admin, so reads can't leak another owner's config by guessing ids.
    guards::require_config_owner_or_admin(&dbc, actor, config_id).await?;
    let data = podcast_config_get::handle(&dbc, config_id).await?;
    Ok(Json(ResponseData::from_data(data)))
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
