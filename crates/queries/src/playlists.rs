use halogen_orm::episode::{Column as EpisodeColumn, Entity as EpisodeEntity};
use halogen_orm::user_episode_status as ues_entity;
use halogen_wire::{FilterParams, PlaybackStatus, ValidationErrors};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QuerySelect};
/// Filter IDs from an already-authorized playlist using the actor's listen state.
pub async fn filter_episodes(
    dbc: &sea_orm::DatabaseConnection,
    user_id: i32,
    episode_ids: &[i32],
    filter: &FilterParams,
) -> Result<Vec<i32>, ValidationErrors> {
    let episodes = EpisodeEntity::find()
        .filter(EpisodeColumn::Id.is_in(episode_ids.to_vec()))
        .all(dbc)
        .await
        .map_err(halogen_wire::DbValidationErrors::from)?;

    let mut filtered: Vec<i32> = Vec::new();
    for ep in episodes {
        let mut pass = true;
        if let Some(ref search) = filter.search {
            let search_term = search.trim_start_matches('%').trim_end_matches('%');
            if !ep.title.contains(search_term) {
                pass = false;
            }
        }
        if let Some(podcast_id) = filter.podcast_id
            && ep.podcast_id != podcast_id
        {
            pass = false;
        }
        if let Some(ref download_status) = filter.download_status
            && ep.download_status.as_str() != download_status
        {
            pass = false;
        }
        if let Some(published_after) = filter.published_after {
            if let Some(pub_at) = ep.published_at {
                if pub_at < published_after {
                    pass = false;
                }
            } else {
                pass = false;
            }
        }
        if pass {
            filtered.push(ep.id);
        }
    }

    // Per-user playback-status filter. The status is in `user_episode_status` (not
    // on `episode`), so restrict the surviving ids against the caller's rows —
    // mirroring `episode_list`. Episodes with no row count as UNPLAYED.
    if let Some(ref status) = filter.playback_status {
        filtered = restrict_by_playback_status(dbc, user_id, filtered, status).await?;
    }

    Ok(filtered)
}

/// Narrow `episode_ids` to those whose per-user listen state matches `status`
/// (`UNPLAYED`/`PLAYED`/`FINISHED`). An unknown status matches nothing — the same
/// behaviour as a column-equality on a bad value. UNPLAYED keeps episodes with no
/// row (or a row still marked Unplayed); the others require an exact-status row.
async fn restrict_by_playback_status(
    dbc: &sea_orm::DatabaseConnection,
    user_id: i32,
    episode_ids: Vec<i32>,
    status: &str,
) -> Result<Vec<i32>, ValidationErrors> {
    if episode_ids.is_empty() {
        return Ok(episode_ids);
    }
    let wanted = match status {
        "UNPLAYED" => Some(PlaybackStatus::Unplayed),
        "PLAYED" => Some(PlaybackStatus::Played),
        "FINISHED" => Some(PlaybackStatus::Finished),
        _ => None,
    };
    let Some(wanted) = wanted else {
        return Ok(Vec::new());
    };

    if wanted == PlaybackStatus::Unplayed {
        // Drop every episode the caller has marked PLAYED/FINISHED; the rest (no
        // row, or a row still Unplayed) stay.
        let non_unplayed: std::collections::HashSet<i32> = ues_entity::Entity::find()
            .filter(ues_entity::Column::UserId.eq(user_id))
            .filter(ues_entity::Column::EpisodeId.is_in(episode_ids.clone()))
            .filter(ues_entity::Column::PlaybackStatus.ne(PlaybackStatus::Unplayed))
            .select_only()
            .column(ues_entity::Column::EpisodeId)
            .into_tuple::<i32>()
            .all(dbc)
            .await
            .map_err(halogen_wire::DbValidationErrors::from)?
            .into_iter()
            .collect();
        Ok(episode_ids
            .into_iter()
            .filter(|id| !non_unplayed.contains(id))
            .collect())
    } else {
        let matching: std::collections::HashSet<i32> = ues_entity::Entity::find()
            .filter(ues_entity::Column::UserId.eq(user_id))
            .filter(ues_entity::Column::EpisodeId.is_in(episode_ids.clone()))
            .filter(ues_entity::Column::PlaybackStatus.eq(wanted))
            .select_only()
            .column(ues_entity::Column::EpisodeId)
            .into_tuple::<i32>()
            .all(dbc)
            .await
            .map_err(halogen_wire::DbValidationErrors::from)?
            .into_iter()
            .collect();
        Ok(episode_ids
            .into_iter()
            .filter(|id| matching.contains(id))
            .collect())
    }
}
