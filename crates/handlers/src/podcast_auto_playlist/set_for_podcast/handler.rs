use std::collections::HashSet;

use halogen_wire::{PodcastAutoPlaylistData, PodcastAutoPlaylistSetData, ValidationErrors};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, Set, TransactionTrait};
use tracing::info;

use crate::{db_error, not_found};
use halogen_orm::playlist::{Column as PlaylistCol, Entity as PlaylistEntity};
use halogen_orm::podcast::{Column as PodCol, Entity as PodEntity};
use halogen_orm::podcast_auto_playlist::{self, Column as PapCol, Entity as PapEntity};

/// Replace a podcast's full set of auto-add playlists with `playlist_ids`, transactionally. Idempotent — covers
/// both first-time create and later edits (the UI always sends the whole set). Unknown / since-deleted ids, and
/// any playlist not owned by the podcast's owner, are silently dropped so a stale client can't hard-fail. 404
/// if the podcast is gone. Returns the resulting (filtered, deduped) set.
pub async fn handle(
    dbc: &sea_orm::DatabaseConnection,
    podcast_id: i32,
    data: PodcastAutoPlaylistSetData,
) -> Result<Vec<PodcastAutoPlaylistData>, ValidationErrors> {
    // The DTO was already validated by the `Body<PodcastAutoPlaylistSetData>` extractor.
    let txn = dbc
        .begin()
        .await
        .map_err(db_error("beginning transaction"))?;

    // The podcast must exist; its owner scopes which playlists may be linked.
    let owner_id = match PodEntity::find()
        .filter(PodCol::Id.eq(podcast_id))
        .one(&txn)
        .await
        .map_err(db_error("fetching podcast"))?
    {
        Some(p) => p.owner_id,
        None => return Err(not_found("Podcast not found")),
    };

    // Keep only ids that map to a live playlist OWNED BY THE PODCAST OWNER — linking
    // another user's playlist would let the poller inject episodes into it (IDOR).
    // Dedupe, preserving order.
    let live: HashSet<i32> = PlaylistEntity::find()
        .filter(PlaylistCol::UserId.eq(owner_id))
        .all(&txn)
        .await
        .map_err(db_error("listing playlists"))?
        .into_iter()
        .map(|p| p.id)
        .collect();
    let mut seen: HashSet<i32> = HashSet::new();
    let target: Vec<i32> = data
        .playlist_ids
        .into_iter()
        .filter(|id| live.contains(id) && seen.insert(*id))
        .collect();

    // Replace semantics: wipe the existing set, insert the new one.
    PapEntity::delete_many()
        .filter(PapCol::PodcastId.eq(podcast_id))
        .exec(&txn)
        .await
        .map_err(db_error("deleting auto-playlist links"))?;

    let now = chrono::Utc::now();
    for &playlist_id in &target {
        let model = podcast_auto_playlist::ActiveModel {
            podcast_id: Set(podcast_id),
            playlist_id: Set(playlist_id),
            add_to_start: Set(data.add_to_start),
            created_at: Set(now),
            updated_at: Set(now),
        };
        PapEntity::insert(model)
            .exec(&txn)
            .await
            .map_err(db_error("inserting auto-playlist link"))?;
    }

    txn.commit()
        .await
        .map_err(db_error("committing transaction"))?;
    info!(
        "Set {} auto-playlist(s) for podcast {}",
        target.len(),
        podcast_id
    );

    Ok(target
        .into_iter()
        .map(|playlist_id| PodcastAutoPlaylistData {
            podcast_id,
            playlist_id,
            add_to_start: data.add_to_start,
        })
        .collect())
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
