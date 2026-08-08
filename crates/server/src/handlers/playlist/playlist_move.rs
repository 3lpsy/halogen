use sea_orm::{ColumnTrait, EntityTrait, Order, QueryFilter, QueryOrder};
use tracing::info;

use crate::handlers::playlist::order;
use crate::handlers::{db_error, not_found};
use halogen_orm::playlist::{self, Column, Entity as PlaylistEntity};
use halogen_wire::ValidationErrors;

/// Move `playlist_id` to index `to` within its owner's playlists, then rewrite
/// every row's `position` to its new contiguous index (0..n). This self-heals any
/// holes left by deletes. `to` is clamped into range; an out-of-range or unchanged
/// target is a no-op. Mirrors `episode_playlist::handle_move`.
///
/// Scopes by the playlist's OWNER (not the caller), so an admin reordering another
/// user's playlist reorders within that user's set. The router's owner-or-admin
/// guard authorizes the call.
pub async fn handle_move(
    dbc: &sea_orm::DatabaseConnection,
    playlist_id: i32,
    to: i32,
) -> Result<(), ValidationErrors> {
    let now = chrono::Utc::now();
    let target = PlaylistEntity::find_by_id(playlist_id)
        .one(dbc)
        .await
        .map_err(db_error("loading playlist for move"))?
        .ok_or_else(|| not_found("Playlist not found"))?;
    let user_id = target.user_id;

    let rows = PlaylistEntity::find()
        .filter(Column::UserId.eq(user_id))
        .order_by(Column::Position, Order::Asc)
        .all(dbc)
        .await
        .map_err(db_error("fetching playlists for move"))?;

    let from = rows
        .iter()
        .position(|r| r.id == playlist_id)
        .ok_or_else(|| not_found("Playlist not found"))?;

    // Reorder the playlist ids, then rewrite positions 0..n in a transaction. An
    // out-of-range or unchanged target is a no-op.
    let ids: Vec<i32> = rows.iter().map(|r| r.id).collect();
    let Some(reordered) = order::reordered(&ids, from, to) else {
        return Ok(());
    };

    order::rewrite_in_txn(dbc, &reordered, |pid, new_pos| playlist::ActiveModel {
        id: sea_orm::ActiveValue::Unchanged(pid),
        name: sea_orm::ActiveValue::NotSet,
        description: sea_orm::ActiveValue::NotSet,
        user_id: sea_orm::ActiveValue::NotSet,
        is_default: sea_orm::ActiveValue::NotSet,
        on_remove_delete_file_server: sea_orm::ActiveValue::NotSet,
        on_remove_delete_file_client: sea_orm::ActiveValue::NotSet,
        position: sea_orm::ActiveValue::Set(new_pos),
        created_at: sea_orm::ActiveValue::NotSet,
        updated_at: sea_orm::ActiveValue::Set(now),
    })
    .await?;

    info!(user_id, playlist_id, to, "Moved playlist");
    Ok(())
}
