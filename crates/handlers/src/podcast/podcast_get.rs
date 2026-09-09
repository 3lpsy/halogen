use halogen_wire::{PodcastData, PodcastInclude, ValidationErrors};
use sea_orm::EntityTrait;
use tracing::info;

use crate::{db_error, wants};
use halogen_orm::common::EntityHelpers;
use halogen_orm::podcast::Entity as PodcastEntity;
use halogen_orm::podcast_config as podcast_config_entity;

pub async fn handle(
    dbc: &sea_orm::DatabaseConnection,
    podcast_id: i32,
    includes: Option<&Vec<PodcastInclude>>,
) -> Result<PodcastData, ValidationErrors> {
    let podcast = PodcastEntity::by_id_or_err(dbc, podcast_id).await?;

    let load_podcast_config = wants(includes, PodcastInclude::PodcastConfig);

    // Capture the `Copy` fk before moving the model into `PodcastData` (no clone).
    let podcast_config_id = podcast.podcast_config_id;
    let mut data = PodcastData::from(podcast);

    if load_podcast_config && let Some(config_id) = podcast_config_id {
        let config = podcast_config_entity::Entity::find_by_id(config_id)
            .one(dbc)
            .await
            .map_err(db_error("fetching podcast config"))?;
        data.podcast_config = config.map(|c| c.into());
    }

    data.episode_count = super::podcast_list::episode_counts(dbc, &[podcast_id])
        .await?
        .get(&podcast_id)
        .copied();

    info!("Fetched podcast '{}'", data.title);
    Ok(data)
}
