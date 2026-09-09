use crate::routers::extractors::{Actor, Id};
use crate::routers::guards;
use axum::{Extension, Json};
use halogen_wire::ResponseData;
use sea_orm::DatabaseConnection;
use tracing::warn;

use crate::handlers::playlist::playlist_delete::handle as playlist_delete;
use crate::routers::ApiError;

pub async fn delete(
    Extension(dbc): Extension<DatabaseConnection>,
    actor: Actor,
    Id(playlist_id): Id,
) -> Result<Json<ResponseData<()>>, ApiError> {
    guards::require_playlist_owner_or_admin(&dbc, actor, playlist_id).await?;
    // Owner deletes are scoped to their own rows; admins (guard-approved) skip the
    // owner filter so they can delete any playlist.
    let owner_scope = if actor.is_admin { None } else { Some(actor.id) };
    playlist_delete(&dbc, owner_scope, playlist_id)
        .await
        .map_err(|err| {
            warn!("Error deleting playlist: {:?}", err);
            err
        })?;

    Ok(Json(ResponseData::from_data(())))
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
