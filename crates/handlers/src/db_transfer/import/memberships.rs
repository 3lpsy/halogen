use crate::db_error;
use halogen_orm::episode_playlist;
use halogen_wire::{DbImportSummaryData, ValidationErrors};
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseTransaction, EntityTrait, QueryFilter, Set};
use std::collections::HashMap;

pub(super) async fn merge(
    txn: &DatabaseTransaction,
    src_links: &[episode_playlist::Model],
    playlist_map: &HashMap<i32, i32>,
    episode_map: &HashMap<i32, i32>,
    summary: &mut DbImportSummaryData,
) -> Result<(), ValidationErrors> {
    // Per target playlist: (existing episode ids, next free position).
    let mut membership: HashMap<i32, (std::collections::HashSet<i32>, i32)> = HashMap::new();
    for link in src_links {
        let (Some(&plid), Some(&eid)) = (
            playlist_map.get(&link.playlist_id),
            episode_map.get(&link.episode_id),
        ) else {
            continue;
        };
        if let std::collections::hash_map::Entry::Vacant(e) = membership.entry(plid) {
            let existing = episode_playlist::Entity::find()
                .filter(episode_playlist::Column::PlaylistId.eq(plid))
                .all(txn)
                .await
                .map_err(db_error("reading target playlist membership"))?;
            let next = existing.iter().map(|l| l.position).max().unwrap_or(-1) + 1;
            let ids = existing.into_iter().map(|l| l.episode_id).collect();
            e.insert((ids, next));
        }
        let entry = membership.get_mut(&plid).expect("preloaded above");
        if entry.0.contains(&eid) {
            continue;
        }
        episode_playlist::ActiveModel {
            episode_id: Set(eid),
            playlist_id: Set(plid),
            position: Set(entry.1),
            created_at: Set(link.created_at),
            updated_at: Set(link.updated_at),
        }
        .insert(txn)
        .await
        .map_err(db_error("creating an imported playlist membership"))?;
        entry.0.insert(eid);
        entry.1 += 1;
        summary.playlist_links_created += 1;
    }

    Ok(())
}
