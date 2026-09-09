use std::collections::HashMap;

use crate::{db_error, wants};
use halogen_orm::episode_playlist::{Column as EpColumn, Entity as EpisodePlaylistEntity};
use halogen_wire::{EpisodePlaylistData, PlaylistData, PlaylistInclude, ValidationErrors};
use sea_orm::{ColumnTrait, EntityTrait, Order as SeaOrder, QueryFilter, QueryOrder};

/// Populate `episode_ids` / `episode_playlist` on `playlists` from the
/// `episode_playlist` pivot — one batched query across all the page's playlist
/// ids, ordered by `position` — when those includes were requested. `episode_ids`
/// becomes `Some([])` for an empty playlist (distinct from `None` = not loaded).
pub(crate) async fn attach_episode_includes(
    dbc: &sea_orm::DatabaseConnection,
    playlists: &mut [PlaylistData],
    includes: Option<&Vec<PlaylistInclude>>,
) -> Result<(), ValidationErrors> {
    let want_ids = wants(includes, PlaylistInclude::EpisodeIds);
    let want_pivot = wants(includes, PlaylistInclude::EpisodePlaylist);
    if (!want_ids && !want_pivot) || playlists.is_empty() {
        return Ok(());
    }
    let ids: Vec<i32> = playlists.iter().map(|p| p.id).collect();
    let rows = EpisodePlaylistEntity::find()
        .filter(EpColumn::PlaylistId.is_in(ids))
        .order_by(EpColumn::PlaylistId, SeaOrder::Asc)
        .order_by(EpColumn::Position, SeaOrder::Asc)
        .all(dbc)
        .await
        .map_err(db_error("loading playlist episode pivots"))?;

    let mut by_playlist: HashMap<i32, Vec<_>> = HashMap::new();
    for r in rows {
        by_playlist.entry(r.playlist_id).or_default().push(r);
    }
    for p in playlists.iter_mut() {
        let group = by_playlist.remove(&p.id).unwrap_or_default();
        if want_ids {
            p.episode_ids = Some(group.iter().map(|r| r.episode_id).collect());
        }
        if want_pivot {
            p.episode_playlist = Some(group.into_iter().map(EpisodePlaylistData::from).collect());
        }
    }
    Ok(())
}
