use halogen_wire::{PlaylistData, PlaylistStoreData, ValidationErrors};
use sea_orm::sea_query::Expr;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, TransactionTrait};
use tracing::info;

use crate::handlers::db_error;
use halogen_orm::playlist;
use halogen_orm::playlist::{Column, Entity as PlaylistEntity};
use halogen_utils::constants::*;
use halogen_utils::verrors;

pub async fn handle(
    dbc: &sea_orm::DatabaseConnection,
    user_id: i32,
    request_data: PlaylistStoreData,
) -> Result<PlaylistData, ValidationErrors> {
    // The DTO was already validated by the `Body<PlaylistStoreData>` extractor.
    let now = chrono::Utc::now();
    // Creating directly as the default queue: clear any existing default FIRST,
    // in the same transaction, since the DB enforces a single `is_default` row
    // (partial unique index). Mirrors the update handler's promote-to-default.
    let make_default = request_data.is_default == Some(true);

    // The queue IS the default playlist, and it may not exist yet. While none
    // exists, the FIRST playlist created must become it — so reject a non-default
    // create when there's no default. (The client also locks the checkbox; this is
    // the defensive server-side half.)
    if !make_default {
        let has_default = PlaylistEntity::find()
            .filter(Column::UserId.eq(user_id))
            .filter(Column::IsDefault.eq(true))
            .one(dbc)
            .await
            .map_err(db_error("checking for an existing default"))?
            .is_some();
        if !has_default {
            return Err(verrors(
                "is_default",
                VALIDATION_INVALID_CODE,
                "The first playlist must be the default queue.".to_string(),
            )
            .into());
        }
    }

    let txn = dbc
        .begin()
        .await
        .map_err(db_error("starting playlist create transaction"))?;

    if make_default {
        // Scope to this user: clearing the prior default must not touch another
        // user's default (the DB unique index is per-user).
        PlaylistEntity::update_many()
            .col_expr(Column::IsDefault, Expr::value(false))
            .col_expr(Column::UpdatedAt, Expr::value(now))
            .filter(Column::UserId.eq(user_id))
            .filter(Column::IsDefault.eq(true))
            .exec(&txn)
            .await
            .map_err(db_error("clearing previous default playlist"))?;
    }

    // Append to the end of the user's manual order: position = max + 1 (0 when
    // first). Same idiom as `episode_playlist::handle_store`.
    let position = PlaylistEntity::find()
        .filter(Column::UserId.eq(user_id))
        .all(&txn)
        .await
        .map_err(db_error("fetching playlist positions"))?
        .iter()
        .map(|p| p.position)
        .max()
        .map(|p| p + 1)
        .unwrap_or(0);

    let playlist_model = playlist::ActiveModel {
        id: sea_orm::ActiveValue::NotSet,
        name: sea_orm::ActiveValue::Set(request_data.name),
        description: sea_orm::ActiveValue::Set(request_data.description),
        user_id: sea_orm::ActiveValue::Set(user_id),
        is_default: sea_orm::ActiveValue::Set(make_default),
        position: sea_orm::ActiveValue::Set(position),
        on_remove_delete_file_server: sea_orm::ActiveValue::Set(
            request_data.on_remove_delete_file_server.unwrap_or(false),
        ),
        on_remove_delete_file_client: sea_orm::ActiveValue::Set(
            request_data.on_remove_delete_file_client.unwrap_or(false),
        ),
        created_at: sea_orm::ActiveValue::Set(now),
        updated_at: sea_orm::ActiveValue::Set(now),
    };

    let saved_playlist = playlist_model
        .insert(&txn)
        .await
        .map_err(db_error("inserting playlist"))?;

    txn.commit()
        .await
        .map_err(db_error("committing playlist create"))?;

    let data: PlaylistData = saved_playlist.into();

    info!("Created playlist");
    Ok(data)
}
