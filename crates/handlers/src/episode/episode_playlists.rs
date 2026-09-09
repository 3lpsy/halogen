use halogen_wire::{DefaultListParams, Paginator, PlaylistData, PlaylistInclude, ValidationErrors};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use tracing::info;

use crate::db_error;
use halogen_orm::episode_playlist::{Column as EpColumn, Entity as EpisodePlaylistEntity};
use halogen_orm::playlist::{Column, Entity as PlaylistEntity};

/// List the caller's playlists that contain `episode_id`. Backs the picker's
/// pre-selection ("which playlists is this episode already in?") so it can lazily
/// fetch + render the member playlists even when they aren't on the first page of
/// the paged playlist list.
pub async fn handle(
    dbc: &sea_orm::DatabaseConnection,
    user_id: i32,
    episode_id: i32,
    params: &DefaultListParams<PlaylistInclude>,
) -> Result<(Vec<PlaylistData>, Paginator), ValidationErrors> {
    let pagination = params.pagination.clone().unwrap_or_default();
    let order = params.order.clone().unwrap_or_default();

    // The playlist ids this episode belongs to; the user scoping is applied on the
    // playlist query below (so a member playlist owned by someone else is excluded).
    let member_ids: Vec<i32> = EpisodePlaylistEntity::find()
        .filter(EpColumn::EpisodeId.eq(episode_id))
        .all(dbc)
        .await
        .map_err(db_error("fetching episode playlist membership"))?
        .into_iter()
        .map(|r| r.playlist_id)
        .collect();

    // Empty membership → a never-match filter (id = -1, ids are positive) so we
    // return an empty page with valid SQL instead of `IN ()`.
    let query = PlaylistEntity::find().filter(Column::UserId.eq(user_id));
    let query = if member_ids.is_empty() {
        query.filter(Column::Id.eq(-1))
    } else {
        query.filter(Column::Id.is_in(member_ids))
    };
    let (playlists, paginator) =
        halogen_orm::common::paginate(dbc, query, &pagination, &order).await?;

    let mut data: Vec<PlaylistData> = playlists.into_iter().map(|p| p.into()).collect();
    crate::playlist::attach_episode_includes(dbc, &mut data, params.includes.as_ref()).await?;

    info!(episode_id, "Fetched {} playlists for episode", data.len());
    Ok((data, paginator))
}
