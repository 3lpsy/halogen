use crate::routers::extractors::{Actor, Body, Id};
use crate::routers::guards;
use axum::{Extension, Json};
use halogen_wire::{PlaylistReorderData, ResponseData};
use sea_orm::DatabaseConnection;
use tracing::warn;

use crate::handlers::playlist::playlist_reorder::handle_reorder;
use crate::routers::ApiError;

/// `POST /playlists/{id}/reorder-by` — smart-reorder a playlist's episodes by a
/// chosen field + direction, baking the order into the `Custom` (position)
/// sequence. Owner-or-admin only.
pub async fn reorder_by(
    Extension(dbc): Extension<DatabaseConnection>,
    actor: Actor,
    Id(playlist_id): Id,
    Body(data): Body<PlaylistReorderData>,
) -> Result<Json<ResponseData<()>>, ApiError> {
    guards::require_playlist_owner_or_admin(&dbc, actor, playlist_id).await?;
    handle_reorder(&dbc, playlist_id, data.field, data.direction)
        .await
        .map_err(|err| {
            warn!("Error reordering playlist: {:?}", err);
            err
        })?;

    Ok(Json(ResponseData::from_data(())))
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
