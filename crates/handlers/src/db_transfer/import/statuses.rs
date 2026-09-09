use crate::db_error;
use halogen_orm::user_episode_status;
use halogen_wire::{DbImportSummaryData, ValidationErrors};
use sea_orm::{ActiveModelTrait, DatabaseTransaction, EntityTrait, NotSet, Set};
use std::collections::HashMap;

pub(super) async fn merge(
    txn: &DatabaseTransaction,
    src_statuses: &[user_episode_status::Model],
    user_map: &HashMap<i32, i32>,
    episode_map: &HashMap<i32, i32>,
    summary: &mut DbImportSummaryData,
) -> Result<(), ValidationErrors> {
    // Same preload treatment as playbacks (the other per-user history table).
    let mut target_statuses: HashMap<(i32, i32), user_episode_status::Model> =
        user_episode_status::Entity::find()
            .all(txn)
            .await
            .map_err(db_error("reading target episode statuses"))?
            .into_iter()
            .map(|s| ((s.user_id, s.episode_id), s))
            .collect();
    for st in src_statuses {
        let (Some(&uid), Some(&eid)) = (user_map.get(&st.user_id), episode_map.get(&st.episode_id))
        else {
            continue;
        };
        let existing = target_statuses.get(&(uid, eid)).cloned();
        match existing {
            Some(t) if t.updated_at >= st.updated_at => {}
            Some(t) => {
                let mut am: user_episode_status::ActiveModel = t.into();
                am.playback_status = Set(st.playback_status.clone());
                am.updated_at = Set(st.updated_at);
                let updated = am
                    .update(txn)
                    .await
                    .map_err(db_error("updating an imported episode status"))?;
                target_statuses.insert((uid, eid), updated);
                summary.statuses_upserted += 1;
            }
            None => {
                let created = user_episode_status::ActiveModel {
                    id: NotSet,
                    user_id: Set(uid),
                    episode_id: Set(eid),
                    playback_status: Set(st.playback_status.clone()),
                    created_at: Set(st.created_at),
                    updated_at: Set(st.updated_at),
                }
                .insert(txn)
                .await
                .map_err(db_error("creating an imported episode status"))?;
                target_statuses.insert((uid, eid), created);
                summary.statuses_upserted += 1;
            }
        }
    }

    Ok(())
}
