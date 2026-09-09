use halogen_wire::{PodcastConfigData, PodcastConfigStoreData, ValidationErrors};
use sea_orm::{
    ActiveModelTrait, ActiveValue, ColumnTrait, EntityTrait, QueryFilter, Set, TransactionTrait,
};
use tracing::info;

use crate::{db_error, not_found};
use halogen_orm::podcast::{Column as PodCol, Entity as PodEntity};
use halogen_utils::constants::*;
use halogen_utils::verrors;

/// Create a podcast config AND link it to the podcast, atomically. Backs `POST /podcasts/{id}/config`: one
/// transaction inserts the config row and sets `podcast.podcast_config_id`, so a partial failure can't leave an
/// orphan config — this nested route is the only way to create a config row (there is no standalone create
/// endpoint). Refuses to clobber an existing config — the UI only routes here when the podcast has none.
pub async fn handle(
    dbc: &sea_orm::DatabaseConnection,
    podcast_id: i32,
    store_data: PodcastConfigStoreData,
) -> Result<PodcastConfigData, ValidationErrors> {
    // The DTO was already validated by the `Body<PodcastConfigStoreData>` extractor.
    let txn = dbc
        .begin()
        .await
        .map_err(db_error("beginning transaction"))?;

    let podcast = PodEntity::find()
        .filter(PodCol::Id.eq(podcast_id))
        .one(&txn)
        .await
        .map_err(db_error("fetching podcast"))?
        .ok_or_else(|| not_found("Podcast not found"))?;

    if podcast.podcast_config_id.is_some() {
        return Err(verrors(
            VALIDATION_REQUEST_FIELD,
            VALIDATION_CONFLICT_CODE,
            "Podcast already has a config".to_string(),
        ));
    }

    let mut active: halogen_orm::podcast_config::ActiveModel = store_data.into();
    active.id = ActiveValue::NotSet;
    let model = active
        .insert(&txn)
        .await
        .map_err(db_error("inserting podcast config"))?;

    let mut pod: halogen_orm::podcast::ActiveModel = podcast.into();
    pod.podcast_config_id = Set(Some(model.id));
    pod.update(&txn)
        .await
        .map_err(db_error("linking podcast config"))?;

    txn.commit()
        .await
        .map_err(db_error("committing transaction"))?;

    let response = PodcastConfigData::from(model);
    info!(
        "Created podcast config '{}' for podcast '{}'",
        response.id, podcast_id
    );
    Ok(response)
}
