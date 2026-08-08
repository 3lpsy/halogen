//! Durable poll-job rows (`poll_job`): one row per sync run — manual, on-demand,
//! or a scheduled service tick. The per-podcast outcomes live in the child
//! [`poll_job_podcast`](crate::poll_job_podcast) table.

use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "poll_job")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = true)]
    pub id: i32,
    /// running | completed | failed (wire `PollJobStatus` snake_case token).
    #[sea_orm(not_null)]
    pub status: String,
    /// manual | scheduled (wire `PollJobTrigger` snake_case token).
    #[sea_orm(not_null)]
    pub trigger: String,
    /// Scope of a single-podcast job (`None` = all feeds). Snapshot, not an FK —
    /// history survives podcast deletion.
    pub podcast_id: Option<i32>,
    #[sea_orm(not_null)]
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    #[sea_orm(not_null)]
    pub total_new: i32,
    #[sea_orm(not_null)]
    pub total_updated: i32,
    #[sea_orm(not_null)]
    pub total_errors: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "super::poll_job_podcast::Entity")]
    PollJobPodcast,
}

impl Related<super::poll_job_podcast::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::PollJobPodcast.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
