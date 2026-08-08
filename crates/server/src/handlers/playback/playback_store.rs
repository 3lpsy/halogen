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

use crate::handlers::{db_error, not_found};

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

    // Atomic upsert on the unique (user_id, episode_id) index: a playback row is a
    // resume position updated continuously, so a repeat save moves the cursor on
    // the existing row rather than inserting a duplicate. Doing it as a single
    // `INSERT ... ON CONFLICT DO UPDATE` (returning the row) closes the
    // find-then-insert race where two concurrent saves both miss the row and one
    // then fails the unique index with a 500. `created_at` is intentionally left
    // out of the update set so it survives later saves.
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
        // Skip the write when the status is unchanged (the common case during
        // continuous playback) to avoid churn. Otherwise write it as an atomic
        // `INSERT ... ON CONFLICT DO UPDATE` on the unique (user_id, episode_id)
        // index — the same race-closing pattern as the playback upsert above.
        // The earlier find-then-insert-or-update let two concurrent first saves
        // both miss the row, with the loser's unique-index insert failing (and
        // being swallowed), so the status could be left stale.
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
mod tests {
    use super::*;

    #[test]
    fn playback_status_for_covers_states_and_threshold() {
        use PlaybackStatus::*;
        let pct = 4; // Finished within the last 4%.

        // Explicit completion wins regardless of cursor/duration.
        assert_eq!(playback_status_for(0, true, Some(100), pct), Finished);
        assert_eq!(playback_status_for(0, true, None, pct), Finished);

        // Not started → Unplayed (even with a duration).
        assert_eq!(playback_status_for(0, false, Some(100), pct), Unplayed);

        // Started but no duration → can't compute remaining, so Played.
        assert_eq!(playback_status_for(50, false, None, pct), Played);
        assert_eq!(playback_status_for(50, false, Some(0), pct), Played);

        // Threshold boundary on a 100s episode: remaining ≤ 4% → Finished.
        assert_eq!(playback_status_for(96, false, Some(100), pct), Finished); // 4% left
        assert_eq!(playback_status_for(100, false, Some(100), pct), Finished); // 0% left
        assert_eq!(playback_status_for(95, false, Some(100), pct), Played); // 5% left
        assert_eq!(playback_status_for(1, false, Some(100), pct), Played); // just started
    }
}
