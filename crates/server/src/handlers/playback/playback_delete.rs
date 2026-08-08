use halogen_wire::ValidationErrors;
use sea_orm::{ColumnTrait, EntityTrait, ModelTrait, QueryFilter};
use tracing::info;

use crate::handlers::{db_error, not_found};
use halogen_orm::playback::{Column, Entity as PlaybackEntity};

pub async fn handle(
    dbc: &sea_orm::DatabaseConnection,
    user_id: i32,
    id: i32,
) -> Result<(), ValidationErrors> {
    let playback = PlaybackEntity::find_by_id(id)
        .filter(Column::UserId.eq(user_id))
        .one(dbc)
        .await
        .map_err(db_error("fetching playback"))?
        .ok_or_else(|| not_found("Playback not found"))?;

    playback
        .delete(dbc)
        .await
        .map_err(db_error("deleting playback"))?;

    info!("Deleted playback {}", id);
    Ok(())
}
