//! Per-user episode listen-state lookups. Per-user listen state lives in the `user_episode_status` table (not
//! a global `episode.playback_status` column), so the episode read handlers (`list`/`get`/`update`) overwrite
//! the `EpisodeData.playback_status` (which `From<Model>` defaults to Unplayed) with the caller's row. Absence
//! of a row means UNPLAYED.

use std::collections::HashMap;

use halogen_orm::user_episode_status as ues_entity;
use halogen_wire::PlaybackStatus;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QuerySelect};

/// The caller's listen state for each of `episode_ids`, as a map. Episodes with
/// no row are omitted (callers treat a miss as the default, Unplayed).
pub async fn user_status_map(
    dbc: &DatabaseConnection,
    user_id: i32,
    episode_ids: &[i32],
) -> HashMap<i32, PlaybackStatus> {
    if episode_ids.is_empty() {
        return HashMap::new();
    }
    ues_entity::Entity::find()
        .filter(ues_entity::Column::UserId.eq(user_id))
        .filter(ues_entity::Column::EpisodeId.is_in(episode_ids.to_vec()))
        .select_only()
        .column(ues_entity::Column::EpisodeId)
        .column(ues_entity::Column::PlaybackStatus)
        .into_tuple::<(i32, PlaybackStatus)>()
        .all(dbc)
        .await
        .unwrap_or_default()
        .into_iter()
        .collect()
}

/// The caller's listen state for a single episode, defaulting to Unplayed when
/// there is no row.
pub async fn user_status_for(
    dbc: &DatabaseConnection,
    user_id: i32,
    episode_id: i32,
) -> PlaybackStatus {
    ues_entity::Entity::find()
        .filter(ues_entity::Column::UserId.eq(user_id))
        .filter(ues_entity::Column::EpisodeId.eq(episode_id))
        .select_only()
        .column(ues_entity::Column::PlaybackStatus)
        .into_tuple::<PlaybackStatus>()
        .one(dbc)
        .await
        .ok()
        .flatten()
        .unwrap_or_default()
}
