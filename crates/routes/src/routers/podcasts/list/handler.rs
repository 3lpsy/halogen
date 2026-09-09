use axum::{Extension, Json};
use halogen_wire::{DefaultListParams, PodcastData, PodcastInclude, ResponseData};
use sea_orm::DatabaseConnection;

use crate::handlers::podcast::podcast_list;
use crate::routers::ApiError;
use crate::routers::extractors::{AuthUserId, Query};

pub async fn list(
    Extension(dbc): Extension<DatabaseConnection>,
    AuthUserId(user_id): AuthUserId,
    Query(params): Query<DefaultListParams<PodcastInclude>>,
) -> Result<Json<ResponseData<Vec<PodcastData>>>, ApiError> {
    let (podcasts, paginator) = podcast_list::handle(&dbc, user_id, &params)
        .await
        .map_err(ApiError)?;
    Ok(Json(ResponseData::from_paginator(podcasts, paginator)))
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
