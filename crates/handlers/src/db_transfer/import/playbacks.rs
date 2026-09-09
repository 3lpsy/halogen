use crate::db_error;
use halogen_orm::playback;
use halogen_wire::{DbImportSummaryData, ValidationErrors};
use sea_orm::{ActiveModelTrait, DatabaseTransaction, EntityTrait, NotSet, Set};
use std::collections::HashMap;

pub(super) async fn merge(
    txn: &DatabaseTransaction,
    src_playbacks: &[playback::Model],
    user_map: &HashMap<i32, i32>,
    episode_map: &HashMap<i32, i32>,
    summary: &mut DbImportSummaryData,
) -> Result<(), ValidationErrors> {
    // Preload existing playbacks once, keyed by (user, episode) — history is
    // the biggest table an import carries; per-row point queries would make
    // the merge O(rows) round-trips inside the transaction.
    let mut target_playbacks: HashMap<(i32, i32), playback::Model> = playback::Entity::find()
        .all(txn)
        .await
        .map_err(db_error("reading target playbacks"))?
        .into_iter()
        .map(|p| ((p.user_id, p.episode_id), p))
        .collect();
    for pb in src_playbacks {
        let (Some(&uid), Some(&eid)) = (user_map.get(&pb.user_id), episode_map.get(&pb.episode_id))
        else {
            continue;
        };
        let existing = target_playbacks.get(&(uid, eid)).cloned();
        match existing {
            Some(t) if t.updated_at >= pb.updated_at => {}
            Some(t) => {
                let mut am: playback::ActiveModel = t.into();
                am.cursor = Set(pb.cursor);
                am.completed = Set(pb.completed);
                am.updated_at = Set(pb.updated_at);
                let updated = am
                    .update(txn)
                    .await
                    .map_err(db_error("updating an imported playback"))?;
                target_playbacks.insert((uid, eid), updated);
                summary.playbacks_upserted += 1;
            }
            None => {
                let created = playback::ActiveModel {
                    id: NotSet,
                    user_id: Set(uid),
                    episode_id: Set(eid),
                    cursor: Set(pb.cursor),
                    completed: Set(pb.completed),
                    created_at: Set(pb.created_at),
                    updated_at: Set(pb.updated_at),
                }
                .insert(txn)
                .await
                .map_err(db_error("creating an imported playback"))?;
                target_playbacks.insert((uid, eid), created);
                summary.playbacks_upserted += 1;
            }
        }
    }

    Ok(())
}
