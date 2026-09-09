use axum::{Extension, Json};
use halogen_wire::{PlaybackData, PlaybackListParams, ResponseData};
use sea_orm::DatabaseConnection;
use tracing::warn;

use crate::handlers::playback::playback_list::handle as playback_list;
use crate::routers::ApiError;
use crate::routers::extractors::{AuthUserId, Query};

pub async fn list(
    Extension(dbc): Extension<DatabaseConnection>,
    AuthUserId(user_id): AuthUserId,
    Query(params): Query<PlaybackListParams>,
) -> Result<Json<ResponseData<Vec<PlaybackData>>>, ApiError> {
    let (playback_data, paginator) =
        playback_list(&dbc, user_id, &params).await.map_err(|err| {
            warn!("Error fetching playbacks: {:?}", err);
            err
        })?;

    Ok(Json(ResponseData::from_paginator(playback_data, paginator)))
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
