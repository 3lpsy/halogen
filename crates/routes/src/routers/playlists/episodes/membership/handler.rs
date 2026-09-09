use crate::routers::extractors::{Actor, Body, Ids2};
use crate::routers::guards;
use axum::{Extension, Json};
use halogen_wire::{
    EpisodePlaylistData, EpisodePlaylistMoveData, EpisodePlaylistStoreData, ResponseData,
};
use sea_orm::DatabaseConnection;
use tracing::warn;

use crate::handlers::playlist::episode_playlist::{
    handle_delete as episode_playlist_delete, handle_move as episode_playlist_move,
    handle_store as episode_playlist_store, server_delete_flag,
};
use crate::routers::ApiError;

pub async fn store(
    Extension(dbc): Extension<DatabaseConnection>,
    actor: Actor,
    Ids2(playlist_id, episode_id): Ids2,
    Body(store_data): Body<EpisodePlaylistStoreData>,
) -> Result<Json<ResponseData<EpisodePlaylistData>>, ApiError> {
    // Mutating a playlist's membership requires owning the playlist (or admin).
    guards::require_playlist_owner_or_admin(&dbc, actor, playlist_id).await?;
    // The episode must also be one the caller may access (subscribed / owner / admin), matching the bulk-add
    // path — otherwise a user could add an episode from a podcast they can't see into their own playlist and
    // read its metadata back via the playlist listing. (Remove/move don't gate on subscription: the caller must
    // always be able to manage what's already in their own playlist.)
    guards::require_episode_subscribed(&dbc, actor, episode_id).await?;
    // The PATH ids are authoritative — they're what the guards authorized. The body
    // carries no ids (only `position`), so a caller physically can't target a
    // different (e.g. another user's) playlist/episode than the guard checked.
    let data = episode_playlist_store(&dbc, playlist_id, episode_id, store_data.position)
        .await
        .map_err(|err| {
            warn!("Error creating episode-playlist: {:?}", err);
            err
        })?;

    Ok(Json(ResponseData::from_data(data)))
}

pub async fn delete(
    Extension(dbc): Extension<DatabaseConnection>,
    actor: Actor,
    Ids2(playlist_id, episode_id): Ids2,
) -> Result<Json<ResponseData<()>>, ApiError> {
    guards::require_playlist_owner_or_admin(&dbc, actor, playlist_id).await?;
    // Per-playlist cleanup flag: when set, the service drops the server-side
    // download once the episode belongs to no other playlist.
    let delete_server_file = server_delete_flag(&dbc, playlist_id).await?;
    episode_playlist_delete(&dbc, episode_id, playlist_id, delete_server_file)
        .await
        .map_err(|err| {
            warn!("Error deleting episode-playlist: {:?}", err);
            err
        })?;

    Ok(Json(ResponseData::from_data(())))
}

pub async fn move_(
    Extension(dbc): Extension<DatabaseConnection>,
    actor: Actor,
    Ids2(playlist_id, episode_id): Ids2,
    Body(data): Body<EpisodePlaylistMoveData>,
) -> Result<Json<ResponseData<()>>, ApiError> {
    guards::require_playlist_owner_or_admin(&dbc, actor, playlist_id).await?;
    episode_playlist_move(&dbc, playlist_id, episode_id, data.to)
        .await
        .map_err(|err| {
            warn!("Error moving episode-playlist: {:?}", err);
            err
        })?;

    Ok(Json(ResponseData::from_data(())))
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
