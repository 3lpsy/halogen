use axum::{Extension, Json};
use halogen_wire::{DefaultPlaylistData, ResponseData};
use sea_orm::DatabaseConnection;
use tracing::warn;

use crate::handlers::playlist::playlist_default::handle as playlist_default;
use crate::routers::ApiError;
use crate::routers::extractors::AuthUserId;

/// `GET /playlists/default` — the user's queue (default playlist) or `null`.
pub async fn default(
    Extension(dbc): Extension<DatabaseConnection>,
    AuthUserId(user_id): AuthUserId,
) -> Result<Json<ResponseData<DefaultPlaylistData>>, ApiError> {
    let data = playlist_default(&dbc, user_id).await.map_err(|err| {
        warn!("Error fetching default playlist: {:?}", err);
        err
    })?;

    Ok(Json(ResponseData::from_data(data)))
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
