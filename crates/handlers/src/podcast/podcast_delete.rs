use halogen_wire::ValidationErrors;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, TransactionTrait};
use tracing::info;

use crate::episode::delete_episode_dependents;
use crate::{db_error, not_found};
use halogen_orm::episode::{Column as EpCol, Entity as EpEntity};
use halogen_orm::podcast::{Column as PodCol, Entity as PodEntity};
use halogen_orm::podcast_auto_playlist::{Column as PodAutoPlCol, Entity as PodAutoPlEntity};
use halogen_orm::user_podcast::{Column as UpCol, Entity as UpEntity};

/// Delete podcast dependents in order, then the podcast, in one transaction. Explicitly clear episodes, memberships,
/// playback/listen state, auto-playlist links, and subscriptions despite foreign-key cascades; partial failures roll
/// back together.
pub async fn handle(
    dbc: &sea_orm::DatabaseConnection,
    podcast_id: i32,
) -> Result<(), ValidationErrors> {
    let txn = dbc
        .begin()
        .await
        .map_err(db_error("beginning transaction"))?;

    // Confirm the podcast exists (404 otherwise — this is also the existence check
    // for admins, who bypass the ownership guard's lookup) and grab its title for
    // the log line below.
    let title = PodEntity::find_by_id(podcast_id)
        .one(&txn)
        .await
        .map_err(db_error("fetching podcast for deletion"))?
        .ok_or_else(|| not_found("Podcast not found"))?
        .title;

    // Episodes belonging to the podcast.
    let episode_ids: Vec<i32> = EpEntity::find()
        .filter(EpCol::PodcastId.eq(podcast_id))
        .all(&txn)
        .await
        .map_err(db_error("fetching episodes for podcast"))?
        .into_iter()
        .map(|e| e.id)
        .collect();

    // Each episode's dependents (memberships/playbacks/listen-state), shared with
    // `episode_delete`, then the episode rows themselves.
    delete_episode_dependents(&txn, &episode_ids).await?;
    EpEntity::delete_many()
        .filter(EpCol::PodcastId.eq(podcast_id))
        .exec(&txn)
        .await
        .map_err(db_error("deleting episodes"))?;

    // The podcast's auto-add playlist links.
    PodAutoPlEntity::delete_many()
        .filter(PodAutoPlCol::PodcastId.eq(podcast_id))
        .exec(&txn)
        .await
        .map_err(db_error("deleting auto-playlist links"))?;

    // Subscriptions to this podcast.
    UpEntity::delete_many()
        .filter(UpCol::PodcastId.eq(podcast_id))
        .exec(&txn)
        .await
        .map_err(db_error("deleting subscriptions"))?;

    // Finally the podcast row.
    PodEntity::delete_many()
        .filter(PodCol::Id.eq(podcast_id))
        .exec(&txn)
        .await
        .map_err(db_error("deleting podcast"))?;

    txn.commit()
        .await
        .map_err(db_error("committing transaction"))?;

    info!(
        "Deleted podcast '{}' and {} episode(s) + their playbacks/memberships",
        title,
        episode_ids.len()
    );
    Ok(())
}
