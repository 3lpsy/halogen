use halogen_wire::{PodcastData, PodcastStoreData, ValidationErrors};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set, TransactionTrait};
use tracing::info;

use crate::handlers::db_error;
use halogen_orm::podcast::{Column, Entity as PodcastEntity};
use halogen_orm::user_podcast;

/// Create or subscribe-to a podcast.
///
/// `feed_url` is globally unique — there is one shared podcast row per feed. If a
/// podcast with this feed already exists, the caller is simply **subscribed to it
/// as a non-owner** (the `owner_id` stays the original creator) and the existing
/// row is returned. Otherwise a new podcast is created owned by the caller and the
/// caller is auto-subscribed. Idempotent and transactional.
pub async fn handle(
    dbc: &sea_orm::DatabaseConnection,
    owner_id: i32,
    store_data: PodcastStoreData,
) -> Result<PodcastData, ValidationErrors> {
    // The DTO was already validated by the `Body<PodcastStoreData>` extractor.
    let txn = dbc
        .begin()
        .await
        .map_err(db_error("beginning transaction"))?;

    // One shared row per feed: reuse an existing podcast, else create it.
    let existing = PodcastEntity::find()
        .filter(Column::FeedUrl.eq(store_data.feed_url.clone()))
        .one(&txn)
        .await
        .map_err(db_error("fetching existing podcast"))?;

    let model = match existing {
        Some(p) => p,
        None => {
            let mut active: halogen_orm::podcast::ActiveModel = store_data.into();
            active.id = sea_orm::ActiveValue::NotSet;
            active.owner_id = Set(owner_id);
            active
                .insert(&txn)
                .await
                .map_err(db_error("inserting podcast"))?
        }
    };

    // Subscribe the caller (idempotent — skip if a row already exists).
    let already = user_podcast::Entity::find_by_id((owner_id, model.id))
        .one(&txn)
        .await
        .map_err(db_error("fetching subscription"))?
        .is_some();
    if !already {
        let now = chrono::Utc::now();
        let sub = user_podcast::ActiveModel {
            user_id: Set(owner_id),
            podcast_id: Set(model.id),
            created_at: Set(now),
            updated_at: Set(now),
        };
        sub.insert(&txn)
            .await
            .map_err(db_error("inserting subscription"))?;
    }

    txn.commit()
        .await
        .map_err(db_error("committing transaction"))?;

    info!(
        "Stored podcast '{}' (owner {}, subscriber {})",
        model.title, model.owner_id, owner_id
    );
    Ok(PodcastData::from(model))
}
