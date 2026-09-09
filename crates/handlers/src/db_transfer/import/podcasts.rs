use crate::db_error;
use halogen_orm::{podcast, podcast_config};
use halogen_wire::{DbImportSummaryData, ValidationErrors};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseTransaction, EntityTrait, NotSet, QueryFilter, Set,
};
use std::collections::HashMap;
use tracing::warn;

pub(super) async fn merge(
    txn: &DatabaseTransaction,
    src_podcasts: &[podcast::Model],
    src_configs: &HashMap<i32, podcast_config::Model>,
    user_map: &HashMap<i32, i32>,
    summary: &mut DbImportSummaryData,
) -> Result<(HashMap<i32, i32>, Vec<i32>), ValidationErrors> {
    let mut podcast_map: HashMap<i32, i32> = HashMap::new();
    // Podcasts whose target row already existed — their episodes need matching
    // instead of blind insertion.
    let mut merged_podcasts: Vec<i32> = Vec::new();
    for sp in src_podcasts {
        let Some(&owner) = user_map.get(&sp.owner_id) else {
            warn!(podcast = sp.id, "Skipping podcast with unmapped owner");
            continue;
        };
        // Match globally unique feed_url alone, even when another owner added it. Reuse the shared row and attach
        // subscriptions later; matching owner too would attempt a duplicate feed insert and roll back the import.
        let existing = podcast::Entity::find()
            .filter(podcast::Column::FeedUrl.eq(sp.feed_url.clone()))
            .one(txn)
            .await
            .map_err(db_error("matching an imported podcast"))?;
        match existing {
            Some(t) => {
                podcast_map.insert(sp.id, t.id);
                merged_podcasts.push(sp.id);
                summary.podcasts_merged += 1;
            }
            None => {
                // Clone the per-podcast config first (fresh id), if any.
                let config_id = match sp.podcast_config_id.and_then(|id| src_configs.get(&id)) {
                    Some(cfg) => Some(
                        podcast_config::ActiveModel {
                            id: NotSet,
                            poll_interval_seconds: Set(cfg.poll_interval_seconds),
                            max_episodes: Set(cfg.max_episodes),
                            max_concurrent_downloads: Set(cfg.max_concurrent_downloads),
                            auto_download_enabled: Set(cfg.auto_download_enabled),
                            created_at: Set(cfg.created_at),
                            updated_at: Set(cfg.updated_at),
                        }
                        .insert(txn)
                        .await
                        .map_err(db_error("creating an imported podcast config"))?
                        .id,
                    ),
                    None => None,
                };
                let created = podcast::ActiveModel {
                    id: NotSet,
                    title: Set(sp.title.clone()),
                    description: Set(sp.description.clone()),
                    feed_url: Set(sp.feed_url.clone()),
                    art_url: Set(sp.art_url.clone()),
                    // Machine-local: the art cache file doesn't travel.
                    art_file_path: Set(None),
                    author: Set(sp.author.clone()),
                    etag: Set(sp.etag.clone()),
                    last_modified: Set(sp.last_modified.clone()),
                    polled_at: Set(sp.polled_at),
                    podcast_config_id: Set(config_id),
                    owner_id: Set(owner),
                    feed_url_redirects: Set(sp.feed_url_redirects.clone()),
                    created_at: Set(sp.created_at),
                    updated_at: Set(sp.updated_at),
                }
                .insert(txn)
                .await
                .map_err(db_error("creating an imported podcast"))?;
                podcast_map.insert(sp.id, created.id);
                summary.podcasts_created += 1;
            }
        }
    }

    Ok((podcast_map, merged_podcasts))
}
