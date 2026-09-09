use halogen_wire::{PlaylistData, PlaylistUpdateData, ValidationErrors};
use sea_orm::sea_query::Expr;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set, TransactionTrait};
use tracing::info;

use crate::{db_error, not_found};
use halogen_orm::playlist::{Column, Entity as PlaylistEntity};

pub async fn handle(
    dbc: &sea_orm::DatabaseConnection,
    playlist_id: i32,
    request_data: PlaylistUpdateData,
) -> Result<PlaylistData, ValidationErrors> {
    // Look up by id only — the router's ownership guard has already authorized the
    // actor (owner or admin). The owner's id is taken from `existing` below so the
    // default-clear is scoped to the playlist's actual owner (correct even when an
    // admin edits another user's playlist).
    let existing = PlaylistEntity::find()
        .filter(Column::Id.eq(playlist_id))
        .one(dbc)
        .await
        .map_err(db_error("fetching playlist"))?
        .ok_or_else(|| not_found("Playlist not found"))?;

    let now = chrono::Utc::now();
    let txn = dbc
        .begin()
        .await
        .map_err(db_error("starting playlist update transaction"))?;

    // Promoting this playlist to the default queue: clear any *other* current
    // default FIRST, in the same transaction. The DB enforces a single
    // `is_default` row (partial unique index), so the old default must be unset
    // before this one is set or the write would violate the index.
    if request_data.is_default == Some(true) {
        // Scope to the playlist's owner: clearing the prior default must not touch
        // another user's default (the DB unique index is per-user).
        PlaylistEntity::update_many()
            .col_expr(Column::IsDefault, Expr::value(false))
            .col_expr(Column::UpdatedAt, Expr::value(now))
            .filter(Column::UserId.eq(existing.user_id))
            .filter(Column::IsDefault.eq(true))
            .filter(Column::Id.ne(playlist_id))
            .exec(&txn)
            .await
            .map_err(db_error("clearing previous default playlist"))?;
    }

    let mut playlist: halogen_orm::playlist::ActiveModel = existing.into();

    if let Some(name) = request_data.name {
        playlist.name = Set(name);
    }

    if let Some(description) = request_data.description {
        playlist.description = Set(Some(description));
    }

    if let Some(is_default) = request_data.is_default {
        playlist.is_default = Set(is_default);
    }

    if let Some(v) = request_data.on_remove_delete_file_server {
        playlist.on_remove_delete_file_server = Set(v);
    }

    if let Some(v) = request_data.on_remove_delete_file_client {
        playlist.on_remove_delete_file_client = Set(v);
    }

    playlist.updated_at = Set(now);

    // `update` returns the updated row — capture it, then commit the transaction.
    let updated = playlist
        .update(&txn)
        .await
        .map_err(db_error("updating playlist"))?;

    txn.commit()
        .await
        .map_err(db_error("committing playlist update"))?;

    info!("Updated playlist '{}'", updated.name);
    Ok(PlaylistData::from(updated))
}
