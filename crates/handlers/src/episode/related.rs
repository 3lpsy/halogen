use std::collections::HashMap;

use sea_orm::{ColumnTrait, DatabaseConnection, DatabaseTransaction, EntityTrait, QueryFilter};

use crate::db_error;
use halogen_orm::episode_chapter::{Column as EchCol, Entity as EchEntity};
use halogen_orm::episode_playlist::{Column as EpPlCol, Entity as EpPlEntity};
use halogen_orm::playback::{Column as PbCol, Entity as PbEntity};
use halogen_orm::podcast as podcast_entity;
use halogen_orm::user_episode_status::{Column as UesCol, Entity as UesEntity};
use halogen_wire::{EpisodeData, ValidationErrors};

/// Delete everything that hangs off a set of episodes — playlist memberships, playback history, per-user listen
/// state, and chapters — within `txn`. The shared core of the episode cleanup `episode_delete` (one episode)
/// and `podcast_delete` (all of a podcast's episodes) both perform; FKs would cascade these, but we delete them
/// explicitly for deterministic ordering. No-op on an empty slice (an empty `IN ()` is degenerate SQL).
pub async fn delete_episode_dependents(
    txn: &DatabaseTransaction,
    episode_ids: &[i32],
) -> Result<(), ValidationErrors> {
    if episode_ids.is_empty() {
        return Ok(());
    }
    let ids = episode_ids.iter().copied();

    EpPlEntity::delete_many()
        .filter(EpPlCol::EpisodeId.is_in(ids.clone()))
        .exec(txn)
        .await
        .map_err(db_error("deleting playlist memberships"))?;

    PbEntity::delete_many()
        .filter(PbCol::EpisodeId.is_in(ids.clone()))
        .exec(txn)
        .await
        .map_err(db_error("deleting playback history"))?;

    UesEntity::delete_many()
        .filter(UesCol::EpisodeId.is_in(ids.clone()))
        .exec(txn)
        .await
        .map_err(db_error("deleting listen state"))?;

    EchEntity::delete_many()
        .filter(EchCol::EpisodeId.is_in(ids))
        .exec(txn)
        .await
        .map_err(db_error("deleting chapters"))?;

    Ok(())
}

/// Embed each episode's parent podcast (when `load`): fetch every referenced
/// podcast in one query and map it onto `EpisodeData.podcast` by id. The shared
/// include-loader the episode read endpoints (single + list) and the
/// playlist-episodes list would otherwise open-code. No-op when not requested or empty.
pub async fn attach_podcasts(
    dbc: &DatabaseConnection,
    episodes: &mut [EpisodeData],
    load: bool,
) -> Result<(), ValidationErrors> {
    if !load || episodes.is_empty() {
        return Ok(());
    }
    let podcast_ids: Vec<i32> = episodes.iter().map(|e| e.podcast_id).collect();
    let podcasts: HashMap<i32, _> = podcast_entity::Entity::find()
        .filter(podcast_entity::Column::Id.is_in(podcast_ids))
        .all(dbc)
        .await
        .map_err(db_error("loading podcasts for episodes"))?
        .into_iter()
        .map(|p| (p.id, p))
        .collect();
    for ep in episodes.iter_mut() {
        ep.podcast = podcasts.get(&ep.podcast_id).cloned().map(Into::into);
    }
    Ok(())
}
