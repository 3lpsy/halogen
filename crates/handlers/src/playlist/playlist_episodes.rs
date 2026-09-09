use std::collections::HashMap;

use halogen_wire::{DefaultListParams, EpisodeData, EpisodeInclude, Paginator, ValidationErrors};
use sea_orm::{
    ColumnTrait, EntityTrait, Order as SeaOrder, PaginatorTrait, QueryFilter, QueryOrder,
};
use tracing::info;

use crate::episode::user_status::user_status_map;
use crate::{db_error, not_found, wants};
use halogen_orm::episode::{Column as EpisodeColumn, Entity as EpisodeEntity};
use halogen_orm::episode_playlist::{Column, Entity as EpisodePlaylistEntity};

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
        halogen_queries::playlists::filter_episodes(dbc, user_id, &episode_ids, filter).await?
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

    crate::episode::attach_podcasts(dbc, &mut episodes, load_podcast).await?;

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
