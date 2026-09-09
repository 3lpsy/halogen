//! Resolve EpisodeInclude::Playback from the caller's unique user/episode row. Always scope by user_id to prevent
//! cursor leaks; a missing row yields None. See user_status for the matching listen-state lookup.

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
