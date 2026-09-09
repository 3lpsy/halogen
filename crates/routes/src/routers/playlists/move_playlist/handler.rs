use crate::routers::extractors::{Actor, Body, Id};
use crate::routers::guards;
use axum::{Extension, Json};
use halogen_wire::{PlaylistMoveData, ResponseData};
use sea_orm::DatabaseConnection;
use tracing::warn;

use crate::handlers::playlist::playlist_move::handle_move;
use crate::routers::ApiError;

/// `POST /playlists/{id}/move` — move a playlist within its owner's manual
/// order. Owner-or-admin only; the handler scopes to the playlist's owner.
pub async fn move_playlist(
    Extension(dbc): Extension<DatabaseConnection>,
    actor: Actor,
    Id(playlist_id): Id,
    Body(data): Body<PlaylistMoveData>,
) -> Result<Json<ResponseData<()>>, ApiError> {
    guards::require_playlist_owner_or_admin(&dbc, actor, playlist_id).await?;
    handle_move(&dbc, playlist_id, data.to)
        .await
        .map_err(|err| {
            warn!("Error moving playlist: {:?}", err);
            err
        })?;

    Ok(Json(ResponseData::from_data(())))
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
