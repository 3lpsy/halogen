use halogen_wire::{PlaybackData, ValidationErrors};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use tracing::info;

use crate::{db_error, not_found};
use halogen_orm::playback::{Column, Entity as PlaybackEntity};

pub async fn handle(
    dbc: &sea_orm::DatabaseConnection,
    user_id: i32,
    playback_id: i32,
) -> Result<PlaybackData, ValidationErrors> {
    let playback = PlaybackEntity::find()
        .filter(Column::Id.eq(playback_id))
        .filter(Column::UserId.eq(user_id))
        .one(dbc)
        .await
        .map_err(db_error("fetching playback"))?
        .ok_or_else(|| not_found("Playback not found"))?;

    info!("Fetched playback {}", playback.id);
    Ok(playback.into())
}
