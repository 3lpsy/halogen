//! Per-podcast outcome rows for a poll job (`poll_job_podcast`), child of
//! [`poll_job`](crate::poll_job). Mirrors the wire `PodcastPollResultData`;
//! `podcast_id`/`title` are snapshots (no FK) so run history survives podcast
//! deletion.

use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "poll_job_podcast")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = true)]
    pub id: i32,
    #[sea_orm(not_null)]
    pub poll_job_id: i32,
    #[sea_orm(not_null)]
    pub podcast_id: i32,
    #[sea_orm(not_null)]
    pub title: String,
    /// polled | skipped | error (wire `PodcastPollOutcome` snake_case token).
    #[sea_orm(not_null)]
    pub outcome: String,
    #[sea_orm(not_null)]
    pub new_episodes: i32,
    #[sea_orm(not_null)]
    pub updated_episodes: i32,
    #[sea_orm(not_null)]
    pub errors: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::poll_job::Entity",
        from = "Column::PollJobId",
        to = "super::poll_job::Column::Id"
    )]
    PollJob,
}

impl Related<super::poll_job::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::PollJob.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
