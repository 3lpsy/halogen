use halogen_wire::{PodcastData, PodcastUpdateData, ValidationErrors};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use tracing::info;

use crate::handlers::{db_error, not_found};
use halogen_orm::podcast::{Column, Entity as PodcastEntity};

pub async fn handle(
    dbc: &sea_orm::DatabaseConnection,
    podcast_id: i32,
    update_data: PodcastUpdateData,
) -> Result<PodcastData, ValidationErrors> {
    let existing = PodcastEntity::find()
        .filter(Column::Id.eq(podcast_id))
        .one(dbc)
        .await
        .map_err(db_error("fetching podcast for update"))?
        .ok_or_else(|| not_found("Podcast not found"))?;

    let mut podcast: halogen_orm::podcast::ActiveModel = existing.into();
    if let Some(new_title) = update_data.title {
        podcast.title = Set(new_title);
    }
    if let Some(new_description) = update_data.description {
        podcast.description = Set(new_description);
    }
    if let Some(new_feed_url) = update_data.feed_url {
        podcast.feed_url = Set(new_feed_url);
    }
    // `art_url` / `author` are not updatable via the API (feed-derived; see
    // `PodcastUpdateData`), so they're left untouched here.

    // `update` returns the updated row — no refetch needed.
    let updated = podcast
        .update(dbc)
        .await
        .map_err(db_error("updating podcast"))?;

    info!("Updated podcast '{}'", updated.title);
    Ok(PodcastData::from(updated))
}
