use halogen_wire::ValidationErrors;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set, TransactionTrait};
use tracing::info;

use crate::{db_error, not_found};
use halogen_orm::podcast::{Column as PodCol, Entity as PodEntity};
use halogen_orm::podcast_config::{Column as CfgCol, Entity as CfgEntity};

/// Unlink and delete a podcast's config, atomically (revert to global defaults). Backs `DELETE
/// /podcasts/{id}/config`: one transaction nulls `podcast.podcast_config_id` then deletes the config row, so no
/// podcast is ever left pointing at a deleted config — this nested route is the only way to delete a config
/// row, and it always nulls the FK first. Idempotent: a podcast with no config is a no-op success.
pub async fn handle(
    dbc: &sea_orm::DatabaseConnection,
    podcast_id: i32,
) -> Result<(), ValidationErrors> {
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

    if let Some(config_id) = podcast.podcast_config_id {
        // Unlink first so no row references the config we're about to delete.
        let mut pod: halogen_orm::podcast::ActiveModel = podcast.into();
        pod.podcast_config_id = Set(None);
        pod.update(&txn)
            .await
            .map_err(db_error("unlinking podcast config"))?;

        CfgEntity::delete_many()
            .filter(CfgCol::Id.eq(config_id))
            .exec(&txn)
            .await
            .map_err(db_error("deleting podcast config"))?;

        info!(
            "Removed podcast config '{}' from podcast '{}'",
            config_id, podcast_id
        );
    }

    txn.commit()
        .await
        .map_err(db_error("committing transaction"))?;
    Ok(())
}
