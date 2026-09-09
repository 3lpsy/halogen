use crate::db_error;
use halogen_orm::{episode, episode_chapter};
use halogen_wire::{DbImportSummaryData, ValidationErrors};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseTransaction, EntityTrait, NotSet, QueryFilter, Set,
};
use std::collections::HashMap;

struct EpisodeMatches {
    by_guid: HashMap<String, i32>,
    by_url: HashMap<String, i32>,
}

pub(super) async fn merge(
    txn: &DatabaseTransaction,
    src_episodes: &[episode::Model],
    src_chapters: &[episode_chapter::Model],
    podcast_map: &HashMap<i32, i32>,
    merged_podcasts: &[i32],
    summary: &mut DbImportSummaryData,
) -> Result<HashMap<i32, i32>, ValidationErrors> {
    let mut episode_map: HashMap<i32, i32> = HashMap::new();
    // Per merged target podcast: existing episodes keyed by guid and by
    // content_url, preloaded once.
    let mut target_eps: HashMap<i32, EpisodeMatches> = HashMap::new();
    for src_pid in merged_podcasts {
        let tgt_pid = podcast_map[src_pid];
        let existing = episode::Entity::find()
            .filter(episode::Column::PodcastId.eq(tgt_pid))
            .all(txn)
            .await
            .map_err(db_error("reading target episodes for matching"))?;
        let mut by_guid = HashMap::new();
        let mut by_url = HashMap::new();
        for e in existing {
            if let Some(g) = &e.guid {
                by_guid.insert(g.clone(), e.id);
            }
            by_url.insert(e.content_url.clone(), e.id);
        }
        target_eps.insert(tgt_pid, EpisodeMatches { by_guid, by_url });
    }
    for se in src_episodes {
        let Some(&tgt_pid) = podcast_map.get(&se.podcast_id) else {
            continue;
        };
        let matched = target_eps.get(&tgt_pid).and_then(|matches| {
            se.guid
                .as_ref()
                .and_then(|g| matches.by_guid.get(g))
                .or_else(|| matches.by_url.get(&se.content_url))
                .copied()
        });
        match matched {
            Some(tid) => {
                episode_map.insert(se.id, tid);
                summary.episodes_merged += 1;
            }
            None => {
                let created = episode::ActiveModel {
                    id: NotSet,
                    podcast_id: Set(tgt_pid),
                    title: Set(se.title.clone()),
                    description: Set(se.description.clone()),
                    content_url: Set(se.content_url.clone()),
                    guid: Set(se.guid.clone()),
                    art_url: Set(se.art_url.clone()),
                    published_at: Set(se.published_at),
                    // Defense in depth (exports are already scrubbed): nothing
                    // imported may claim bytes on this machine's disk.
                    downloaded_at: Set(None),
                    content_file_path: Set(None),
                    download_size: Set(None),
                    art_file_path: Set(None),
                    download_status: Set(halogen_wire::DownloadStatus::NotDownloaded),
                    download_started_at: Set(None),
                    download_attempts: Set(0),
                    duration_secs: Set(se.duration_secs),
                    created_at: Set(se.created_at),
                    updated_at: Set(se.updated_at),
                }
                .insert(txn)
                .await
                .map_err(db_error("creating an imported episode"))?;
                episode_map.insert(se.id, created.id);
                summary.episodes_created += 1;
                if let Some(matches) = target_eps.get_mut(&tgt_pid) {
                    if let Some(g) = &se.guid {
                        matches.by_guid.insert(g.clone(), created.id);
                    }
                    matches.by_url.insert(se.content_url.clone(), created.id);
                }
                // Chapters ride only with episodes we created (matched ones
                // keep the target's).
                for ch in src_chapters.iter().filter(|c| c.episode_id == se.id) {
                    episode_chapter::ActiveModel {
                        id: NotSet,
                        episode_id: Set(created.id),
                        title: Set(ch.title.clone()),
                        starts_at_secs: Set(ch.starts_at_secs),
                        created_at: Set(ch.created_at),
                        updated_at: Set(ch.updated_at),
                    }
                    .insert(txn)
                    .await
                    .map_err(db_error("creating an imported chapter"))?;
                    summary.chapters_created += 1;
                }
            }
        }
    }

    Ok(episode_map)
}
