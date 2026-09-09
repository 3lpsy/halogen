use crate::routers::extractors::{Actor, Body, Id};
use crate::routers::guards;
use axum::{Extension, Json};
use halogen_wire::{PlaylistData, PlaylistUpdateData, ResponseData};
use sea_orm::DatabaseConnection;
use tracing::warn;

use crate::handlers::playlist::playlist_update::handle as playlist_update;
use crate::routers::ApiError;

pub async fn update(
    Extension(dbc): Extension<DatabaseConnection>,
    actor: Actor,
    Id(playlist_id): Id,
    Body(update_data): Body<PlaylistUpdateData>,
) -> Result<Json<ResponseData<PlaylistData>>, ApiError> {
    // After body validation, before mutating: owner-or-admin only.
    guards::require_playlist_owner_or_admin(&dbc, actor, playlist_id).await?;
    let data = playlist_update(&dbc, playlist_id, update_data)
        .await
        .map_err(|err| {
            warn!("Error updating playlist: {:?}", err);
            err
        })?;

    Ok(Json(ResponseData::from_data(data)))
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
