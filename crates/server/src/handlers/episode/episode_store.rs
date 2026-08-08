use halogen_wire::{EpisodeData, EpisodeStoreData, ValidationErrors};
use sea_orm::ActiveModelTrait;
use tracing::info;

use crate::handlers::db_error;
use halogen_orm::episode::ActiveModel as EpisodeActiveModel;

pub async fn handle(
    dbc: &sea_orm::DatabaseConnection,
    store_data: EpisodeStoreData,
) -> Result<EpisodeData, ValidationErrors> {
    // The DTO was already validated by the `Body<EpisodeStoreData>` extractor.
    let mut active: EpisodeActiveModel = store_data.into();
    active.id = sea_orm::ActiveValue::NotSet;

    let model = active
        .insert(dbc)
        .await
        .map_err(db_error("inserting episode"))?;
    info!("Created episode '{}'", model.title);
    Ok(EpisodeData::from(model))
}
