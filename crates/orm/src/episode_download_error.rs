//! Episode download-failure rows (`episode_download_error`): one row per failed
//! media download with the failure reason. FK'd to the episode (CASCADE), so
//! errors die with their episode. Shares its shape with
//! [`podcast_sync_error`](crate::podcast_sync_error).

use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "episode_download_error")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = true)]
    pub id: i32,
    #[sea_orm(not_null)]
    pub episode_id: i32,
    /// Human-readable failure reason (`DownloadFailure` display text).
    #[sea_orm(column_type = "Text", not_null)]
    pub reason: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::episode::Entity",
        from = "Column::EpisodeId",
        to = "super::episode::Column::Id"
    )]
    Episode,
}

impl Related<super::episode::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Episode.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}

/// Keep at most this many error rows per episode; older ones are pruned on each
/// insert, so a retry loop can't grow the table unbounded.
pub const MAX_PER_EPISODE: u64 = 20;

impl Entity {
    /// Persist one failure reason for `episode_id`, pruning that episode's
    /// history past [`MAX_PER_EPISODE`]. A best-effort trail — callers log and
    /// swallow the error (the episode row already carries the terminal status).
    pub async fn record(
        dbc: &DatabaseConnection,
        episode_id: i32,
        reason: String,
    ) -> Result<(), DbErr> {
        use sea_orm::ActiveValue::{NotSet, Set};
        use sea_orm::{QueryOrder, QuerySelect};

        let now = Utc::now();
        ActiveModel {
            id: NotSet,
            episode_id: Set(episode_id),
            reason: Set(reason),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(dbc)
        .await?;

        let keep: Vec<i32> = Entity::find()
            .filter(Column::EpisodeId.eq(episode_id))
            .order_by_desc(Column::Id)
            .limit(MAX_PER_EPISODE)
            .all(dbc)
            .await?
            .into_iter()
            .map(|r| r.id)
            .collect();
        // Fewer rows than the cap → nothing beyond `keep` to delete.
        if keep.len() == MAX_PER_EPISODE as usize {
            Entity::delete_many()
                .filter(Column::EpisodeId.eq(episode_id))
                .filter(Column::Id.is_not_in(keep))
                .exec(dbc)
                .await?;
        }
        Ok(())
    }
}
