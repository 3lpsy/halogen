use std::collections::HashSet;

use halogen_wire::{PodcastAutoPlaylistData, ValidationErrors};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

use crate::handlers::db_error;
use halogen_orm::playlist::{Column as PlaylistCol, Entity as PlaylistEntity};
use halogen_orm::podcast_auto_playlist::{Column as PapCol, Entity as PapEntity};

/// List the playlists a podcast auto-adds new episodes to.
///
/// Stale links (the playlist was deleted) are filtered out by intersecting with
/// the live playlist ids, so the result only ever references existing playlists —
/// nothing hard-fails even if a cleanup was ever missed.
pub async fn handle(
    dbc: &sea_orm::DatabaseConnection,
    podcast_id: i32,
) -> Result<Vec<PodcastAutoPlaylistData>, ValidationErrors> {
    let rows = PapEntity::find()
        .filter(PapCol::PodcastId.eq(podcast_id))
        .all(dbc)
        .await
        .map_err(db_error("listing podcast auto-playlists"))?;

    // Only the referenced playlists need an existence check — look those up rather
    // than loading the whole playlist table.
    let referenced: Vec<i32> = rows.iter().map(|r| r.playlist_id).collect();
    let live: HashSet<i32> = if referenced.is_empty() {
        HashSet::new()
    } else {
        PlaylistEntity::find()
            .filter(PlaylistCol::Id.is_in(referenced))
            .all(dbc)
            .await
            .map_err(db_error("listing playlists"))?
            .into_iter()
            .map(|p| p.id)
            .collect()
    };

    Ok(rows
        .into_iter()
        .filter(|r| live.contains(&r.playlist_id))
        .map(PodcastAutoPlaylistData::from)
        .collect())
}
