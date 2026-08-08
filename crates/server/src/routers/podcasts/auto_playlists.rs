//! `GET` / `PUT /podcasts/{id}/auto-playlists` — read and replace the set of
//! playlists a podcast auto-adds new episodes to.
//!
//! `PUT` is a full set-replace (idempotent), so one endpoint covers both
//! first-time configuration and later edits — the form never needs a create vs
//! update distinction. The RSS poller (`halogen_rss::manager`) reads this set
//! and appends each newly-ingested episode to every linked playlist.

use axum::{Extension, Json};
use halogen_wire::{PodcastAutoPlaylistData, PodcastAutoPlaylistSetData, ResponseData};
use sea_orm::DatabaseConnection;

use crate::handlers::podcast_auto_playlist::{get_for_podcast, set_for_podcast};
use crate::routers::ApiError;
use crate::routers::extractors::{Actor, Body, Id};
use crate::routers::guards;

/// `GET /podcasts/{id}/auto-playlists` — the podcast's auto-add playlists.
pub async fn get(
    Extension(dbc): Extension<DatabaseConnection>,
    actor: Actor,
    Id(podcast_id): Id,
) -> Result<Json<ResponseData<Vec<PodcastAutoPlaylistData>>>, ApiError> {
    guards::require_podcast_config_writer(&dbc, actor, podcast_id).await?;
    let rows = get_for_podcast::handle(&dbc, podcast_id).await?;
    Ok(Json(ResponseData::from_data(rows)))
}

/// `PUT /podcasts/{id}/auto-playlists` — replace the podcast's auto-add set.
pub async fn set(
    Extension(dbc): Extension<DatabaseConnection>,
    actor: Actor,
    Id(podcast_id): Id,
    Body(data): Body<PodcastAutoPlaylistSetData>,
) -> Result<Json<ResponseData<Vec<PodcastAutoPlaylistData>>>, ApiError> {
    guards::require_podcast_config_writer(&dbc, actor, podcast_id).await?;
    let rows = set_for_podcast::handle(&dbc, podcast_id, data).await?;
    Ok(Json(ResponseData::from_data(rows)))
}
