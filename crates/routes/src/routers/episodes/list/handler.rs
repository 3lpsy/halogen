use axum::{Extension, Json};
use halogen_wire::{DefaultListParams, EpisodeData, EpisodeInclude, ResponseData};
use sea_orm::DatabaseConnection;

use crate::handlers::episode::episode_list;
use crate::routers::ApiError;
use crate::routers::extractors::{AuthUserId, Query};

/// Fetch episodes with pagination, ordering, and includes. Episodes are scoped to the caller's subscriptions
/// (the handler filters by `user_podcast`), and `playback_status` is per-user, so we thread the caller's id
/// into the handler.
pub async fn list(
    Extension(dbc): Extension<DatabaseConnection>,
    AuthUserId(user_id): AuthUserId,
    Query(params): Query<DefaultListParams<EpisodeInclude>>,
) -> Result<Json<ResponseData<Vec<EpisodeData>>>, ApiError> {
    let (episodes, paginator) = episode_list::handle(&dbc, user_id, &params)
        .await
        .map_err(ApiError)?;
    Ok(Json(ResponseData::from_paginator(episodes, paginator)))
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
