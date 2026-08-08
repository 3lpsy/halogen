use std::collections::HashMap;

use halogen_wire::{
    DefaultListParams, EpisodeData, EpisodeInclude, FilterParams, Paginator, PlaybackStatus,
    ValidationErrors,
};
use sea_orm::{
    ColumnTrait, EntityTrait, Order as SeaOrder, PaginatorTrait, QueryFilter, QueryOrder,
    QuerySelect,
};
use tracing::info;

use crate::handlers::episode::user_status::user_status_map;
use crate::handlers::{db_error, not_found, wants};
use halogen_orm::episode::{Column as EpisodeColumn, Entity as EpisodeEntity};
use halogen_orm::episode_playlist::{Column, Entity as EpisodePlaylistEntity};
use halogen_orm::user_episode_status as ues_entity;

pub async fn handle(
    dbc: &sea_orm::DatabaseConnection,
    user_id: i32,
    playlist_id: i32,
    params: &DefaultListParams<EpisodeInclude>,
) -> Result<(Vec<EpisodeData>, Paginator), ValidationErrors> {
    let pagination = params.pagination.clone().unwrap_or_default();
    // A playlist's natural order is its pivot POSITION: with no order param
    // at all, use it. `unwrap_or_default()` first would fill order_by = "id"
    // (Order::default()) and make the is_empty branch below unreachable —
    // that id-order default is why iOS queues showed front-adds at the end.
    let order_missing = params.order.is_none();
    let order = params.order.clone().unwrap_or_default();
    let load_podcast = wants(params.includes.as_ref(), EpisodeInclude::Podcast);

    let sea_order: SeaOrder = order.direction.clone().into();
    let order_by = order.order_by.as_str();
    let use_position = order_missing || order_by == "position" || order_by.is_empty();

    let playlist_exists = halogen_orm::playlist::Entity::find()
        .filter(halogen_orm::playlist::Column::Id.eq(playlist_id))
        .one(dbc)
        .await
        .map_err(db_error("fetching playlist"))?;

    if playlist_exists.is_none() {
        return Err(not_found("Playlist not found"));
    }

    let ep_pls = EpisodePlaylistEntity::find()
        .filter(Column::PlaylistId.eq(playlist_id))
        .all(dbc)
        .await
        .map_err(db_error("loading playlist episodes"))?;

    let position_map: HashMap<i32, i32> = ep_pls
        .iter()
        .map(|ep_pl| (ep_pl.episode_id, ep_pl.position))
        .collect();
    let episode_ids: Vec<i32> = ep_pls.iter().map(|ep_pl| ep_pl.episode_id).collect();

    let episode_ids = if let Some(ref filter) = params.filter {
        apply_playlist_episode_filters(dbc, user_id, &episode_ids, filter).await?
    } else {
        episode_ids
    };

    // 0-based pagination.
    let size = pagination.size.max(1) as usize;
    let page = pagination.page.max(0) as usize;

    // Position lives on the pivot, not on `episode`, so it can't be a SQL
    // order-by. For position order we sort the FULL id set by position and page
    // that vec — paging by id in SQL would slice the wrong rows for any playlist
    // longer than one page. For episode columns we let SQL order + paginate.
    let (rows, total) = if use_position {
        let mut ordered_ids = episode_ids;
        ordered_ids.sort_by_key(|id| (*position_map.get(id).unwrap_or(&i32::MAX), *id));
        // Honor the requested direction here too — previously the position
        // branch silently returned ascending for Desc.
        if matches!(sea_order, SeaOrder::Desc) {
            ordered_ids.reverse();
        }
        let total = ordered_ids.len();
        let page_ids: Vec<i32> = ordered_ids
            .into_iter()
            .skip(page.saturating_mul(size))
            .take(size)
            .collect();
        let mut rows = EpisodeEntity::find()
            .filter(EpisodeColumn::Id.is_in(page_ids.clone()))
            .all(dbc)
            .await
            .map_err(db_error("loading playlist episodes"))?;
        // `is_in` doesn't preserve order — restore the page's position order.
        let order_index: HashMap<i32, usize> = page_ids
            .iter()
            .enumerate()
            .map(|(i, id)| (*id, i))
            .collect();
        rows.sort_by_key(|e| *order_index.get(&e.id).unwrap_or(&usize::MAX));
        (rows, total)
    } else {
        let column = match order_by {
            "title" => EpisodeColumn::Title,
            "published_at" => EpisodeColumn::PublishedAt,
            "created_at" => EpisodeColumn::CreatedAt,
            "updated_at" => EpisodeColumn::UpdatedAt,
            _ => EpisodeColumn::Id,
        };
        let db_paginator = EpisodeEntity::find()
            .filter(EpisodeColumn::Id.is_in(episode_ids))
            .order_by(column, sea_order)
            .paginate(dbc, size as u64);
        let total = db_paginator
            .num_items()
            .await
            .map_err(db_error("counting playlist episodes"))? as usize;
        let rows = db_paginator
            .fetch_page(page as u64)
            .await
            .map_err(db_error("loading playlist episodes"))?;
        (rows, total)
    };

    let paginator = Paginator {
        page: pagination.page,
        size: pagination.size,
        pages: total.div_ceil(size) as i32,
        total: total as i32,
        order: order.direction,
    };

    let mut episodes: Vec<EpisodeData> = rows.into_iter().map(EpisodeData::from).collect();

    crate::handlers::episode::attach_podcasts(dbc, &mut episodes, load_podcast).await?;

    // Per-user listen state for this page. The status lives in `user_episode_status`
    // now (absence == UNPLAYED); `EpisodeData::from` defaulted it to Unplayed, so
    // overwrite with the caller's row — matching `episode_list` so playlist rows
    // carry the same Finished/Played/Unplayed signal the filter and UI rely on.
    let page_ids: Vec<i32> = episodes.iter().map(|e| e.id).collect();
    let status_map = user_status_map(dbc, user_id, &page_ids).await;
    for e in episodes.iter_mut() {
        e.playback_status = status_map.get(&e.id).cloned().unwrap_or_default();
    }

    info!(
        "Fetched {} episodes for playlist {} (page {}, size {})",
        episodes.len(),
        playlist_id,
        pagination.page,
        pagination.size
    );
    Ok((episodes, paginator))
}

async fn apply_playlist_episode_filters(
    dbc: &sea_orm::DatabaseConnection,
    user_id: i32,
    episode_ids: &[i32],
    filter: &FilterParams,
) -> Result<Vec<i32>, ValidationErrors> {
    let episodes = EpisodeEntity::find()
        .filter(EpisodeColumn::Id.is_in(episode_ids.to_vec()))
        .all(dbc)
        .await
        .map_err(db_error("filtering playlist episodes"))?;

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
            .map_err(db_error("loading played playlist episodes"))?
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
            .map_err(db_error("loading playlist episodes by status"))?
            .into_iter()
            .collect();
        Ok(episode_ids
            .into_iter()
            .filter(|id| matching.contains(id))
            .collect())
    }
}
