//! Admin server-errors read (`GET /admin/server-errors`): the persisted failure
//! histories — podcast RSS sync failures and episode media-download failures —
//! newest first, with entity titles resolved for display.

use std::collections::HashMap;

use axum::{Json, extract::Extension};
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder, QuerySelect};

use halogen_orm::{episode, episode_download_error, podcast, podcast_sync_error};
use halogen_utils::constants::VALIDATION_PANIC_CODE;
use halogen_wire::{
    EpisodeDownloadErrorData, PodcastSyncErrorData, ResponseData, ServerErrorsData,
};

use crate::routers::errors::ApiError;
use crate::routers::extractors::AdminUser;

/// Cap per error kind. The tables are already pruned per entity at write time;
/// this bounds the response for a library with many failing entities.
const MAX_ROWS: u64 = 200;

fn db_err(e: sea_orm::DbErr) -> ApiError {
    ApiError::new("server_errors", VALIDATION_PANIC_CODE, e.to_string())
}

/// GET /admin/server-errors — both failure histories in one read. **Admin only.**
#[axum::debug_handler]
pub async fn get(
    _admin: AdminUser,
    Extension(dbc): Extension<DatabaseConnection>,
) -> Result<Json<ResponseData<ServerErrorsData>>, ApiError> {
    // RSS sync failures, newest first, with the podcast title resolved (None if
    // the podcast vanished — FK cascade makes that only a read-time race).
    let rss_rows = podcast_sync_error::Entity::find()
        .order_by_desc(podcast_sync_error::Column::Id)
        .limit(MAX_ROWS)
        .all(&dbc)
        .await
        .map_err(db_err)?;
    let podcast_ids: Vec<i32> = rss_rows.iter().map(|r| r.podcast_id).collect();
    let podcast_titles: HashMap<i32, String> = podcast::Entity::find()
        .filter(podcast::Column::Id.is_in(podcast_ids))
        .all(&dbc)
        .await
        .map_err(db_err)?
        .into_iter()
        .map(|p| (p.id, p.title))
        .collect();
    let rss_sync = rss_rows
        .into_iter()
        .map(|r| PodcastSyncErrorData {
            id: r.id,
            podcast_id: r.podcast_id,
            podcast_title: podcast_titles.get(&r.podcast_id).cloned(),
            reason: r.reason,
            created_at: r.created_at,
        })
        .collect();

    // Download failures, newest first, with the episode title + parent podcast
    // resolved the same way.
    let dl_rows = episode_download_error::Entity::find()
        .order_by_desc(episode_download_error::Column::Id)
        .limit(MAX_ROWS)
        .all(&dbc)
        .await
        .map_err(db_err)?;
    let episode_ids: Vec<i32> = dl_rows.iter().map(|r| r.episode_id).collect();
    let episodes_by_id: HashMap<i32, (String, i32)> = episode::Entity::find()
        .filter(episode::Column::Id.is_in(episode_ids))
        .all(&dbc)
        .await
        .map_err(db_err)?
        .into_iter()
        .map(|e| (e.id, (e.title, e.podcast_id)))
        .collect();
    let episode_downloads = dl_rows
        .into_iter()
        .map(|r| {
            let found = episodes_by_id.get(&r.episode_id);
            EpisodeDownloadErrorData {
                id: r.id,
                episode_id: r.episode_id,
                episode_title: found.map(|(title, _)| title.clone()),
                podcast_id: found.map(|(_, pid)| *pid),
                reason: r.reason,
                created_at: r.created_at,
            }
        })
        .collect();

    Ok(Json(ResponseData::from_data(ServerErrorsData {
        rss_sync,
        episode_downloads,
    })))
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
