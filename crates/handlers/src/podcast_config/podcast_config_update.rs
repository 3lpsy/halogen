use halogen_wire::{PodcastConfigData, PodcastConfigUpdateData, ValidationErrors};
use sea_orm::{ActiveModelTrait, Set};
use tracing::info;

use crate::db_error;
use halogen_orm::common::EntityHelpers;
use halogen_orm::podcast_config::Entity as PodcastConfigEntity;

pub async fn handle(
    dbc: &sea_orm::DatabaseConnection,
    config_id: i32,
    update_data: PodcastConfigUpdateData,
) -> Result<PodcastConfigData, ValidationErrors> {
    // The DTO was already validated by the `Body<PodcastConfigUpdateData>` extractor.
    let existing = PodcastConfigEntity::by_id_or_err(dbc, config_id).await?;

    let mut config: halogen_orm::podcast_config::ActiveModel = existing.clone().into();

    if let Some(val) = update_data.poll_interval_seconds {
        config.poll_interval_seconds = Set(Some(val));
    }
    if let Some(val) = update_data.max_episodes {
        config.max_episodes = Set(Some(val));
    }
    if let Some(val) = update_data.max_concurrent_downloads {
        config.max_concurrent_downloads = Set(Some(val));
    }
    if let Some(val) = update_data.auto_download_enabled {
        config.auto_download_enabled = Set(Some(val));
    }

    let model = config
        .update(dbc)
        .await
        .map_err(db_error("updating podcast config"))?;

    let response = PodcastConfigData::from(model);
    info!("Updated podcast config '{}'", response.id);
    Ok(response)
}
