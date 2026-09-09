use chrono::Utc;
use halogen_wire::{PlaybackData, PlaybackStatus, PlaybackStoreData, ValidationErrors};
use sea_orm::sea_query::OnConflict;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, Set};
use tracing::{info, warn};

use halogen_orm::episode::Entity as EpisodeEntity;
use halogen_orm::playback::{ActiveModel as PlaybackActiveModel, Column, Entity as PlaybackEntity};
use halogen_orm::user_episode_status::{
    ActiveModel as UesActiveModel, Column as UesColumn, Entity as UesEntity,
};

use crate::{db_error, not_found};

/// Derive the episode's listen state from a playback. `Finished` once playback
/// is within the last `complete_pct`% of the duration (or explicitly completed);
/// `Played` once started; `Unplayed` at the start.
fn playback_status_for(
    cursor: i64,
    completed: bool,
    duration_secs: Option<i32>,
    complete_pct: u16,
) -> PlaybackStatus {
    if completed {
        return PlaybackStatus::Finished;
    }
    if cursor <= 0 {
        return PlaybackStatus::Unplayed;
    }
    if let Some(dur) = duration_secs.filter(|d| *d > 0) {
        let remaining_pct = (dur as f64 - cursor as f64) / dur as f64 * 100.0;
        if remaining_pct <= complete_pct as f64 {
            return PlaybackStatus::Finished;
        }
    }
    PlaybackStatus::Played
}

pub async fn handle(
    dbc: &sea_orm::DatabaseConnection,
    user_id: i32,
    store_data: PlaybackStoreData,
    complete_pct: u16,
) -> Result<PlaybackData, ValidationErrors> {
    // The DTO was already validated by the `Body<PlaybackStoreData>` extractor.
    let episode_id = store_data.episode_id;
    let now = Utc::now();

    // The episode must exist. The FK is enforced at runtime, so a bad id would
    // otherwise trip the constraint and surface as a 500 — pre-checking turns it
    // into a clean 404. Reused below for the listen-state side-effect, so it's not
    // an extra query.
    let episode = EpisodeEntity::find_by_id(episode_id)
        .one(dbc)
        .await
        .map_err(db_error("checking episode"))?
        .ok_or_else(|| not_found("Episode not found"))?;

    // Upsert on the unique user/episode key so concurrent first saves cannot race into a duplicate insert. Later saves
    // update the resume cursor while preserving created_at.
    let active = PlaybackActiveModel {
        id: sea_orm::ActiveValue::NotSet,
        user_id: Set(user_id),
        episode_id: Set(episode_id),
        // Unsigned on the wire, signed in the DB column — saturate rather than wrap.
        cursor: Set(i64::try_from(store_data.cursor).unwrap_or(i64::MAX)),
        completed: Set(store_data.completed),
        created_at: Set(now),
        updated_at: Set(now),
    };
    let playback = PlaybackEntity::insert(active)
        .on_conflict(
            OnConflict::columns([Column::UserId, Column::EpisodeId])
                .update_columns([Column::Cursor, Column::Completed, Column::UpdatedAt])
                .to_owned(),
        )
        .exec_with_returning(dbc)
        .await
        .map_err(db_error("upserting playback"))?;

    // Side-effect: maintain the caller's per-user listen state in
    // `user_episode_status` (one row per user+episode) so the episode list can
    // filter/paginate on it server-side. Best-effort — a failure here doesn't
    // fail the playback save.
    {
        let new_status = playback_status_for(
            playback.cursor,
            playback.completed,
            episode.duration_secs,
            complete_pct,
        );
        let existing_status = UesEntity::find()
            .filter(UesColumn::UserId.eq(user_id))
            .filter(UesColumn::EpisodeId.eq(episode_id))
            .one(dbc)
            .await
            .ok()
            .flatten();
        // Skip unchanged status to avoid continuous-playback writes. Otherwise upsert atomically on user/episode so
        // concurrent first saves cannot leave status stale after a duplicate insert.
        let unchanged = existing_status
            .as_ref()
            .is_some_and(|row| row.playback_status == new_status);
        if !unchanged {
            let am = UesActiveModel {
                id: sea_orm::ActiveValue::NotSet,
                user_id: Set(user_id),
                episode_id: Set(episode_id),
                playback_status: Set(new_status),
                created_at: Set(now),
                updated_at: Set(now),
            };
            if let Err(e) = UesEntity::insert(am)
                .on_conflict(
                    OnConflict::columns([UesColumn::UserId, UesColumn::EpisodeId])
                        .update_columns([UesColumn::PlaybackStatus, UesColumn::UpdatedAt])
                        .to_owned(),
                )
                .exec(dbc)
                .await
            {
                warn!("Failed to upsert user_episode_status: {e}");
            }
        }
    }

    info!(
        "Stored playback {} for episode {} (cursor {})",
        playback.id, playback.episode_id, playback.cursor
    );
    Ok(playback.into())
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
