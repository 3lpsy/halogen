use crate::routers::extractors::{AuthUserId, Body};
use axum::{Extension, Json};
use halogen_wire::{PlaylistData, PlaylistStoreData, ResponseData};
use sea_orm::DatabaseConnection;
use tracing::warn;

use crate::handlers::playlist::playlist_store::handle as playlist_store;
use crate::routers::ApiError;

pub async fn store(
    Extension(dbc): Extension<DatabaseConnection>,
    AuthUserId(user_id): AuthUserId,
    Body(store_data): Body<PlaylistStoreData>,
) -> Result<Json<ResponseData<PlaylistData>>, ApiError> {
    let data = playlist_store(&dbc, user_id, store_data)
        .await
        .map_err(|err| {
            warn!("Error creating playlist: {:?}", err);
            err
        })?;

    Ok(Json(ResponseData::from_data(data)))
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
