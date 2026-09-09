//! Podcast RSS sync-failure rows (`podcast_sync_error`): one row per failed fetch/read/parse with the failure
//! reason. FK'd to the podcast (CASCADE), so errors die with their podcast. Shares its shape with
//! [`episode_download_error`](crate::episode_download_error) — the two error histories differ only in what they
//! FK to.

use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "podcast_sync_error")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = true)]
    pub id: i32,
    #[sea_orm(not_null)]
    pub podcast_id: i32,
    /// Human-readable failure reason (fetch/read/parse error text).
    #[sea_orm(column_type = "Text", not_null)]
    pub reason: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::podcast::Entity",
        from = "Column::PodcastId",
        to = "super::podcast::Column::Id"
    )]
    Podcast,
}

impl Related<super::podcast::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Podcast.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}

/// Keep at most this many error rows per podcast; older ones are pruned on each
/// insert, so an eternally-broken feed can't grow the table unbounded.
pub const MAX_PER_PODCAST: u64 = 20;

impl Entity {
    /// Persist one failure reason for `podcast_id`, pruning that podcast's
    /// history past [`MAX_PER_PODCAST`]. A best-effort trail — callers log and
    /// swallow the error (the sync outcome already carries the error count).
    pub async fn record(
        dbc: &DatabaseConnection,
        podcast_id: i32,
        reason: String,
    ) -> Result<(), DbErr> {
        use sea_orm::ActiveValue::{NotSet, Set};
        use sea_orm::{QueryOrder, QuerySelect};

        let now = Utc::now();
        ActiveModel {
            id: NotSet,
            podcast_id: Set(podcast_id),
            reason: Set(reason),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(dbc)
        .await?;

        let keep: Vec<i32> = Entity::find()
            .filter(Column::PodcastId.eq(podcast_id))
            .order_by_desc(Column::Id)
            .limit(MAX_PER_PODCAST)
            .all(dbc)
            .await?
            .into_iter()
            .map(|r| r.id)
            .collect();
        // Fewer rows than the cap → nothing beyond `keep` to delete.
        if keep.len() == MAX_PER_PODCAST as usize {
            Entity::delete_many()
                .filter(Column::PodcastId.eq(podcast_id))
                .filter(Column::Id.is_not_in(keep))
                .exec(dbc)
                .await?;
        }
        Ok(())
    }
}
