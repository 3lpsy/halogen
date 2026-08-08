//! Per-user playback-cursor lookups for the `EpisodeInclude::Playback` embed.
//!
//! The resume cursor lives in the per-user `playback` table (unique on
//! `(user_id, episode_id)`), so the episode read handlers fill
//! `EpisodeData.playback` with the **caller's** row when the include is
//! requested. Always scoped to `user_id` — a client never sees another user's
//! cursor. Absence of a row means "no saved position" (`None`). Mirrors
//! `user_status.rs`, which does the same for the listen-state pivot.

use std::collections::HashMap;

use halogen_orm::playback as playback_entity;
use halogen_wire::PlaybackData;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};

/// The caller's playback cursor for each of `episode_ids`, as a map. Episodes
/// with no row are omitted (callers treat a miss as "no position").
pub async fn user_playback_map(
    dbc: &DatabaseConnection,
    user_id: i32,
    episode_ids: &[i32],
) -> HashMap<i32, PlaybackData> {
    if episode_ids.is_empty() {
        return HashMap::new();
    }
    playback_entity::Entity::find()
        .filter(playback_entity::Column::UserId.eq(user_id))
        .filter(playback_entity::Column::EpisodeId.is_in(episode_ids.to_vec()))
        .all(dbc)
        .await
        .unwrap_or_default()
        .into_iter()
        // Unique index `(user_id, episode_id)` guarantees one row per episode.
        .map(|m| (m.episode_id, PlaybackData::from(m)))
        .collect()
}

/// The caller's playback cursor for a single episode, or `None` when there is
/// no saved position.
pub async fn user_playback_for(
    dbc: &DatabaseConnection,
    user_id: i32,
    episode_id: i32,
) -> Option<PlaybackData> {
    playback_entity::Entity::find()
        .filter(playback_entity::Column::UserId.eq(user_id))
        .filter(playback_entity::Column::EpisodeId.eq(episode_id))
        .one(dbc)
        .await
        .ok()
        .flatten()
        .map(PlaybackData::from)
}
