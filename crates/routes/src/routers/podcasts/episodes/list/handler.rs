use axum::{Extension, Json};
use halogen_wire::{DefaultListParams, EpisodeData, EpisodeInclude, FilterParams, ResponseData};
use sea_orm::DatabaseConnection;

use crate::handlers::episode::episode_list;
use crate::routers::errors::ApiError;
use crate::routers::extractors::{Actor, Id, Query};
use crate::routers::guards;

/// `GET /podcasts/{id}/episodes` — the nested episode list. This is a thin wrapper that injects the path
/// `podcast_id` into the filter and delegates to the **same** `episode_list::handle` used by `GET /episodes`,
/// so there is exactly one episode-listing implementation.
pub async fn list(
    Extension(dbc): Extension<DatabaseConnection>,
    actor: Actor,
    Id(podcast_id): Id,
    Query(mut params): Query<DefaultListParams<EpisodeInclude>>,
) -> Result<Json<ResponseData<Vec<EpisodeData>>>, ApiError> {
    // Read gate: only subscribers (or owner/admin) may list a podcast's episodes;
    // an unsubscribed or non-existent podcast yields 404 (doesn't leak existence).
    guards::require_subscribed(&dbc, actor, podcast_id).await?;

    let mut filter = params.filter.take().unwrap_or_else(FilterParams::default);
    filter.podcast_id = Some(podcast_id);
    params.filter = Some(filter);

    let (episodes, paginator) = episode_list::handle(&dbc, actor.id, &params)
        .await
        .map_err(ApiError)?;
    Ok(Json(ResponseData::from_paginator(episodes, paginator)))
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
