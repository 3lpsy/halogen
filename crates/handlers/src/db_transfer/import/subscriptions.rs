use crate::db_error;
use halogen_orm::user_podcast;
use halogen_wire::{DbImportSummaryData, ValidationErrors};
use sea_orm::{ActiveModelTrait, DatabaseTransaction, EntityTrait, Set};
use std::collections::HashMap;

pub(super) async fn merge(
    txn: &DatabaseTransaction,
    src_subscriptions: &[user_podcast::Model],
    user_map: &HashMap<i32, i32>,
    podcast_map: &HashMap<i32, i32>,
    summary: &mut DbImportSummaryData,
) -> Result<(), ValidationErrors> {
    // Preload existing (user, podcast) pairs once — batch-match like episodes.
    let mut existing_subs: std::collections::HashSet<(i32, i32)> = user_podcast::Entity::find()
        .all(txn)
        .await
        .map_err(db_error("reading target subscriptions"))?
        .into_iter()
        .map(|s| (s.user_id, s.podcast_id))
        .collect();
    for sub in src_subscriptions {
        let (Some(&uid), Some(&pid)) =
            (user_map.get(&sub.user_id), podcast_map.get(&sub.podcast_id))
        else {
            continue;
        };
        if existing_subs.insert((uid, pid)) {
            user_podcast::ActiveModel {
                user_id: Set(uid),
                podcast_id: Set(pid),
                created_at: Set(sub.created_at),
                updated_at: Set(sub.updated_at),
            }
            .insert(txn)
            .await
            .map_err(db_error("creating an imported subscription"))?;
            summary.subscriptions_created += 1;
        }
    }

    Ok(())
}
