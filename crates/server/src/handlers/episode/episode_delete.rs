use halogen_wire::{EpisodeData, EpisodeDeleteParams, ValidationErrors};
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, TransactionTrait};
use tracing::info;

use crate::handlers::episode::delete_episode_dependents;
use crate::handlers::{db_error, not_found};
use halogen_orm::episode::{Column, Entity as EpisodeEntity};

pub async fn handle(
    dbc: &DatabaseConnection,
    params: &EpisodeDeleteParams,
) -> Result<EpisodeData, ValidationErrors> {
    let episode_id = params.id;

    let episode = EpisodeEntity::find()
        .filter(Column::Id.eq(episode_id))
        .one(dbc)
        .await
        .map_err(db_error("fetching episode"))?
        .ok_or_else(|| not_found("Episode not found"))?;

    let title = episode.title.clone();

    // Delete the episode and everything that depends on it in one transaction.
    // FKs would cascade the dependents, but we clear them explicitly (shared with
    // `podcast_delete` via `delete_episode_dependents`) for deterministic ordered
    // cleanup; the transaction rolls back as a unit.
    let txn = dbc
        .begin()
        .await
        .map_err(db_error("beginning transaction"))?;

    delete_episode_dependents(&txn, &[episode_id]).await?;

    EpisodeEntity::delete_many()
        .filter(Column::Id.eq(episode_id))
        .exec(&txn)
        .await
        .map_err(db_error("deleting episode"))?;

    txn.commit()
        .await
        .map_err(db_error("committing transaction"))?;

    info!("Deleted episode '{}'", title);
    Ok(EpisodeData::from(episode))
}
