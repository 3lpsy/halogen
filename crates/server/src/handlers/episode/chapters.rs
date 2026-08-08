//! Chapter-marker lookups for the `EpisodeInclude::Chapters` embed.
//!
//! Chapters live in the `episode_chapters` table (read-only; written only by the
//! feed-sync path). The episode read handlers fill `EpisodeData.chapters` with the
//! episode's ordered markers when the include is requested. Unlike `user_playback`,
//! chapters are not user-scoped — every caller sees the same set. Mirrors the
//! batched/single shape of `user_playback.rs`.

use std::collections::HashMap;

use halogen_orm::episode_chapter as chapter_entity;
use halogen_wire::EpisodeChapterData;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder};

/// Ordered chapters for each of `episode_ids`, grouped by episode. Episodes with
/// no chapters are omitted (callers treat a miss as "no chapters"). Rows are
/// ordered by `starts_at_secs`, so each episode's vec is in playback order.
pub async fn chapters_map(
    dbc: &DatabaseConnection,
    episode_ids: &[i32],
) -> HashMap<i32, Vec<EpisodeChapterData>> {
    if episode_ids.is_empty() {
        return HashMap::new();
    }
    let rows = chapter_entity::Entity::find()
        .filter(chapter_entity::Column::EpisodeId.is_in(episode_ids.to_vec()))
        .order_by_asc(chapter_entity::Column::EpisodeId)
        .order_by_asc(chapter_entity::Column::StartsAtSecs)
        .all(dbc)
        .await
        .unwrap_or_default();

    let mut map: HashMap<i32, Vec<EpisodeChapterData>> = HashMap::new();
    for row in rows {
        map.entry(row.episode_id)
            .or_default()
            .push(EpisodeChapterData::from(row));
    }
    map
}

/// Ordered chapters for a single episode (empty when it has none).
pub async fn chapters_for(dbc: &DatabaseConnection, episode_id: i32) -> Vec<EpisodeChapterData> {
    chapter_entity::Entity::find()
        .filter(chapter_entity::Column::EpisodeId.eq(episode_id))
        .order_by_asc(chapter_entity::Column::StartsAtSecs)
        .all(dbc)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(EpisodeChapterData::from)
        .collect()
}
