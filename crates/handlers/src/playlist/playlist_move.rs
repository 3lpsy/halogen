use sea_orm::{ColumnTrait, EntityTrait, Order, QueryFilter, QueryOrder};
use tracing::info;

use crate::playlist::order;
use crate::{db_error, not_found};
use halogen_orm::playlist::{self, Column, Entity as PlaylistEntity};
use halogen_wire::ValidationErrors;

/// Move within the playlist owner's set and rewrite contiguous positions to heal gaps. Clamp the target index;
/// unchanged moves are no-ops. The router authorizes owner/admin access, so admin moves still affect the owner's
/// ordering.
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

    // Clamp the target and rewrite positions 0..n transactionally; unchanged moves are a no-op.
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
