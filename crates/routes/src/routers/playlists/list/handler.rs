use axum::{Extension, Json};
use halogen_wire::{DefaultListParams, PlaylistData, PlaylistInclude, ResponseData};
use sea_orm::DatabaseConnection;
use tracing::warn;

use crate::handlers::playlist::playlist_list::handle as playlist_list;
use crate::routers::ApiError;
use crate::routers::extractors::{AuthUserId, Query};

pub async fn list(
    Extension(dbc): Extension<DatabaseConnection>,
    AuthUserId(user_id): AuthUserId,
    Query(params): Query<DefaultListParams<PlaylistInclude>>,
) -> Result<Json<ResponseData<Vec<PlaylistData>>>, ApiError> {
    let (playlist_data, paginator) =
        playlist_list(&dbc, user_id, &params).await.map_err(|err| {
            warn!("Error fetching playlists: {:?}", err);
            err
        })?;

    Ok(Json(ResponseData::from_paginator(playlist_data, paginator)))
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
