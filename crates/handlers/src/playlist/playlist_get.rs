use halogen_wire::{PlaylistData, PlaylistInclude, ValidationErrors};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use tracing::info;

use crate::{db_error, not_found};
use halogen_orm::playlist::{Column, Entity as PlaylistEntity};

pub async fn handle(
    dbc: &sea_orm::DatabaseConnection,
    user_id: i32,
    playlist_id: i32,
    includes: Option<&Vec<PlaylistInclude>>,
) -> Result<PlaylistData, ValidationErrors> {
    // Scope to the caller: another user's playlist must read as not-found so
    // cross-user reads can't see (or probe the existence of) it.
    let playlist = PlaylistEntity::find()
        .filter(Column::Id.eq(playlist_id))
        .filter(Column::UserId.eq(user_id))
        .one(dbc)
        .await
        .map_err(db_error("fetching playlist"))?
        .ok_or_else(|| not_found("Playlist not found"))?;

    let mut data = PlaylistData::from(playlist);
    super::attach_episode_includes(dbc, std::slice::from_mut(&mut data), includes).await?;

    info!("Fetched playlist");
    Ok(data)
}
