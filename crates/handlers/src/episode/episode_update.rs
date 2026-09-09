use halogen_wire::{EpisodeData, EpisodeUpdateData, ValidationErrors};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use tracing::info;

use crate::{db_error, not_found};
use halogen_orm::episode::{Column, Entity as EpisodeEntity};

use super::user_status::user_status_for;

pub async fn handle(
    dbc: &sea_orm::DatabaseConnection,
    user_id: i32,
    episode_id: i32,
    update_data: &EpisodeUpdateData,
) -> Result<EpisodeData, ValidationErrors> {
    let existing = EpisodeEntity::find()
        .filter(Column::Id.eq(episode_id))
        .one(dbc)
        .await
        .map_err(db_error("fetching episode"))?
        .ok_or_else(|| not_found("Episode not found"))?;

    let mut episode: halogen_orm::episode::ActiveModel = existing.clone().into();
    if let Some(new_title) = &update_data.title {
        episode.title = Set(new_title.clone());
    }
    if let Some(new_description) = &update_data.description {
        episode.description = Set(new_description.clone());
    }
    if let Some(new_content_url) = &update_data.content_url {
        episode.content_url = Set(new_content_url.clone());
    }
    if let Some(new_art_url) = &update_data.art_url {
        episode.art_url = Set(Some(new_art_url.clone()));
    }
    if let Some(new_published_at) = &update_data.published_at {
        episode.published_at = Set(Some(*new_published_at));
    }
    // NOTE: download bookkeeping (`downloaded_at`, `content_file_path`,
    // `art_file_path`, `download_status`) is server-managed and not editable via
    // this API — those columns move only through the download pipeline, which
    // writes file paths under `media_root`.

    // `update` returns the updated row — no refetch needed.
    let updated = episode
        .update(dbc)
        .await
        .map_err(db_error("updating episode"))?;

    info!("Updated episode '{}'", updated.title);
    // Reflect the caller's per-user listen state in the response, for parity with
    // the read handlers (the `From<Model>` defaults it to Unplayed).
    let mut data = EpisodeData::from(updated);
    data.playback_status = user_status_for(dbc, user_id, episode_id).await;
    Ok(data)
}
