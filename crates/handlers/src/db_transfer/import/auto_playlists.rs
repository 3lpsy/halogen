use crate::db_error;
use halogen_orm::podcast_auto_playlist;
use halogen_wire::{DbImportSummaryData, ValidationErrors};
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseTransaction, EntityTrait, QueryFilter, Set};
use std::collections::HashMap;

pub(super) async fn merge(
    txn: &DatabaseTransaction,
    src_auto: &[podcast_auto_playlist::Model],
    podcast_map: &HashMap<i32, i32>,
    playlist_map: &HashMap<i32, i32>,
    summary: &mut DbImportSummaryData,
) -> Result<(), ValidationErrors> {
    for ap in src_auto {
        let (Some(&pid), Some(&plid)) = (
            podcast_map.get(&ap.podcast_id),
            playlist_map.get(&ap.playlist_id),
        ) else {
            continue;
        };
        let exists = podcast_auto_playlist::Entity::find()
            .filter(podcast_auto_playlist::Column::PodcastId.eq(pid))
            .filter(podcast_auto_playlist::Column::PlaylistId.eq(plid))
            .one(txn)
            .await
            .map_err(db_error("matching an imported auto-playlist"))?;
        if exists.is_none() {
            podcast_auto_playlist::ActiveModel {
                podcast_id: Set(pid),
                playlist_id: Set(plid),
                add_to_start: Set(ap.add_to_start),
                created_at: Set(ap.created_at),
                updated_at: Set(ap.updated_at),
            }
            .insert(txn)
            .await
            .map_err(db_error("creating an imported auto-playlist"))?;
            summary.auto_playlists_created += 1;
        }
    }

    Ok(())
}
