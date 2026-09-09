use crate::routers::extractors::{AuthUserId, Id, Query};
use axum::{Extension, Json};
use halogen_wire::{PlaylistData, PlaylistShowParams, ResponseData};
use sea_orm::DatabaseConnection;
use tracing::warn;

use crate::handlers::playlist::playlist_get::handle as playlist_get;
use crate::routers::ApiError;

pub async fn get(
    Extension(dbc): Extension<DatabaseConnection>,
    AuthUserId(user_id): AuthUserId,
    Id(playlist_id): Id,
    Query(params): Query<PlaylistShowParams>,
) -> Result<Json<ResponseData<PlaylistData>>, ApiError> {
    let playlist_data = playlist_get(&dbc, user_id, playlist_id, params.includes.as_ref())
        .await
        .map_err(|err| {
            warn!("Error fetching playlist: {:?}", err);
            err
        })?;

    Ok(Json(ResponseData::from_data(playlist_data)))
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
