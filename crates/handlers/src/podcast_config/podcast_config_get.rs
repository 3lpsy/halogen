use halogen_wire::{PodcastConfigData, ValidationErrors};
use tracing::info;

use halogen_orm::common::EntityHelpers;
use halogen_orm::podcast_config::Entity as PodcastConfigEntity;

pub async fn handle(
    dbc: &sea_orm::DatabaseConnection,
    config_id: i32,
) -> Result<PodcastConfigData, ValidationErrors> {
    let config = PodcastConfigEntity::by_id_or_err(dbc, config_id).await?;

    info!("Fetched podcast config '{}'", config.id);
    Ok(PodcastConfigData::from(config))
}
