use halogen_wire::{EpisodePlaylistData, ValidationErrors};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, ModelTrait, Order, PaginatorTrait, QueryFilter,
    QueryOrder,
};
use tracing::{info, warn};

use crate::handlers::playlist::order;
use crate::handlers::{db_error, not_found};
use halogen_orm::episode_playlist::{Column, Entity as EpisodePlaylistEntity};
use halogen_utils::constants::*;
use halogen_utils::verrors;

pub async fn handle_store(
    dbc: &sea_orm::DatabaseConnection,
    playlist_id: i32,
    episode_id: i32,
    position: Option<i32>,
) -> Result<EpisodePlaylistData, ValidationErrors> {
    // The episode must exist. FKs are enforced at runtime (`foreign_keys = ON`), so
    // a bad episode_id would otherwise trip the constraint and surface as a 500 —
    // this check turns that into a clean 404. (The playlist's existence + ownership
    // is checked by the router guard.)
    let episode_exists = halogen_orm::episode::Entity::find_by_id(episode_id)
        .one(dbc)
        .await
        .map_err(db_error("checking episode"))?
        .is_some();
    if !episode_exists {
        return Err(verrors(
            "episode_id",
            VALIDATION_EXISTS_CODE,
            "Episode not found".to_string(),
        ));
    }

    let now = chrono::Utc::now();
    let members = EpisodePlaylistEntity::find()
        .filter(Column::PlaylistId.eq(playlist_id))
        .order_by(Column::Position, Order::Asc)
        .all(dbc)
        .await
        .map_err(db_error("fetching playlist membership"))?;

    // Idempotent: a unique (episode_id, playlist_id) index backs the table, so
    // re-adding an episode already in the playlist returns the existing row rather
    // than tripping the constraint with a 409.
    if let Some(existing) = members.iter().find(|m| m.episode_id == episode_id) {
        return Ok(existing.clone().into());
    }

    // `position` is an insert INDEX: `None` appends (the common path — one cheap
    // insert at max+1); `Some(i)` inserts at `i` (clamped) and renumbers the
    // members 0..n so positions stay contiguous (self-heals any holes), the same
    // invariant `handle_move` maintains.
    let data: EpisodePlaylistData = match position {
        None => {
            let position = members
                .iter()
                .map(|m| m.position)
                .max()
                .map(|p| p + 1)
                .unwrap_or(0);
            let model = halogen_orm::episode_playlist::ActiveModel {
                episode_id: sea_orm::ActiveValue::Set(episode_id),
                playlist_id: sea_orm::ActiveValue::Set(playlist_id),
                position: sea_orm::ActiveValue::Set(position),
                created_at: sea_orm::ActiveValue::Set(now),
                updated_at: sea_orm::ActiveValue::Set(now),
            };
            model
                .insert(dbc)
                .await
                .map_err(db_error("creating episode-playlist association"))?
                .into()
        }
        Some(idx) => {
            // Build the target order (members are position-sorted above) with the new
            // episode at the clamped index.
            let mut order: Vec<i32> = members.iter().map(|m| m.episode_id).collect();
            let at = (idx.max(0) as usize).min(order.len());
            order.insert(at, episode_id);

            // Insert the new row at a free slot past the end (no transient position
            // clash) and renumber every row — including the new one — to its
            // contiguous 0..n position, all in ONE transaction. Doing the insert and
            // the renumber in the same transaction is what keeps a renumber failure
            // from leaving the row committed at the transient position.
            let new_row = halogen_orm::episode_playlist::ActiveModel {
                episode_id: sea_orm::ActiveValue::Set(episode_id),
                playlist_id: sea_orm::ActiveValue::Set(playlist_id),
                position: sea_orm::ActiveValue::Set(members.len() as i32),
                created_at: sea_orm::ActiveValue::Set(now),
                updated_at: sea_orm::ActiveValue::Set(now),
            };
            order::insert_then_rewrite_in_txn(dbc, new_row, &order, |ep_id, new_pos| {
                halogen_orm::episode_playlist::ActiveModel {
                    episode_id: sea_orm::ActiveValue::Unchanged(ep_id),
                    playlist_id: sea_orm::ActiveValue::Unchanged(playlist_id),
                    position: sea_orm::ActiveValue::Set(new_pos),
                    updated_at: sea_orm::ActiveValue::Set(now),
                    created_at: sea_orm::ActiveValue::NotSet,
                }
            })
            .await?;

            EpisodePlaylistData {
                episode_id,
                playlist_id,
                position: at as i32,
            }
        }
    };

    info!("Created episode-playlist association");
    Ok(data)
}

/// Move `episode_id` to index `to` within `playlist_id`, then rewrite every
/// row's `position` to its new contiguous index (0..n). This self-heals any
/// holes left by deletes. `to` is clamped into range; an out-of-range or
/// unchanged target is a no-op.
pub async fn handle_move(
    dbc: &sea_orm::DatabaseConnection,
    playlist_id: i32,
    episode_id: i32,
    to: i32,
) -> Result<(), ValidationErrors> {
    let now = chrono::Utc::now();
    let rows = EpisodePlaylistEntity::find()
        .filter(Column::PlaylistId.eq(playlist_id))
        .order_by(Column::Position, Order::Asc)
        .all(dbc)
        .await
        .map_err(db_error("fetching playlist rows for move"))?;

    let from = rows
        .iter()
        .position(|r| r.episode_id == episode_id)
        .ok_or_else(|| not_found("Episode-playlist association not found"))?;

    // Reorder the episode ids, then rewrite positions 0..n. An out-of-range or
    // unchanged target is a no-op.
    let ids: Vec<i32> = rows.iter().map(|r| r.episode_id).collect();
    let Some(order) = order::reordered(&ids, from, to) else {
        return Ok(());
    };

    rewrite_positions(dbc, playlist_id, &order, now).await?;

    info!(playlist_id, episode_id, to, "Moved episode within playlist");
    Ok(())
}

/// Rewrite a playlist's membership positions to exactly match `ordered_ids`
/// (contiguous 0..n), in a single transaction. Self-heals any holes left by
/// deletes. Shared by the manual move (`handle_move`) and the smart reorder
/// (`playlist_reorder::handle_reorder`) so the position rewrite lives in one place.
pub(crate) async fn rewrite_positions(
    dbc: &sea_orm::DatabaseConnection,
    playlist_id: i32,
    ordered_ids: &[i32],
    now: chrono::DateTime<chrono::Utc>,
) -> Result<(), ValidationErrors> {
    order::rewrite_in_txn(dbc, ordered_ids, |ep_id, new_pos| {
        halogen_orm::episode_playlist::ActiveModel {
            episode_id: sea_orm::ActiveValue::Unchanged(ep_id),
            playlist_id: sea_orm::ActiveValue::Unchanged(playlist_id),
            position: sea_orm::ActiveValue::Set(new_pos),
            updated_at: sea_orm::ActiveValue::Set(now),
            created_at: sea_orm::ActiveValue::NotSet,
        }
    })
    .await
}

/// Load the playlist's `on_remove_delete_file_server` flag. The routers' owner
/// guard only selects `user_id`, so neither delete caller has the row on hand;
/// a missing playlist reads as `false` (the membership lookup 404s on its own).
pub(crate) async fn server_delete_flag(
    dbc: &sea_orm::DatabaseConnection,
    playlist_id: i32,
) -> Result<bool, ValidationErrors> {
    Ok(halogen_orm::playlist::Entity::find_by_id(playlist_id)
        .one(dbc)
        .await
        .map_err(db_error("fetching playlist"))?
        .map(|p| p.on_remove_delete_file_server)
        .unwrap_or(false))
}

pub async fn handle_delete(
    dbc: &sea_orm::DatabaseConnection,
    episode_id: i32,
    playlist_id: i32,
    delete_server_file: bool,
) -> Result<(), ValidationErrors> {
    let episode_playlist_model = EpisodePlaylistEntity::find()
        .filter(Column::EpisodeId.eq(episode_id))
        .filter(Column::PlaylistId.eq(playlist_id))
        .one(dbc)
        .await
        .map_err(db_error("fetching episode-playlist association"))?
        .ok_or_else(|| not_found("Episode-playlist association not found"))?;

    episode_playlist_model
        .delete(dbc)
        .await
        .map_err(db_error("deleting episode-playlist association"))?;

    info!("Deleted episode-playlist association");

    // Per-playlist cleanup (`on_remove_delete_file_server`): drop the server-side
    // download once NO playlist holds the episode — the file is one global
    // per-episode copy shared across users, so another playlist's membership keeps
    // it alive. Best-effort: the membership removal above already succeeded and
    // the bulk route is deliberately lenient per-id, so cleanup never fails the
    // request.
    if delete_server_file {
        match EpisodePlaylistEntity::find()
            .filter(Column::EpisodeId.eq(episode_id))
            .count(dbc)
            .await
        {
            Ok(0) => {
                if let Err(err) = halogen_download::remove_server_download(dbc, episode_id).await {
                    warn!(episode_id, error = ?err, "delete-on-remove: removing server download failed");
                }
            }
            Ok(_) => {}
            Err(err) => {
                warn!(episode_id, error = ?err, "delete-on-remove: counting remaining memberships failed");
            }
        }
    }

    Ok(())
}
