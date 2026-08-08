use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;

use halogen_wire::PlaybackStatus;

/// Per-user listen state for an episode (Unplayed/Played/Finished); replaces the
/// old global `episode.playback_status`. One row per `(user, episode)` — unique
/// index on `(user_id, episode_id)` backs the playback-handler upsert.
#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "user_episode_status")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = true)]
    pub id: i32,
    #[sea_orm(not_null)]
    pub user_id: i32,
    #[sea_orm(not_null)]
    pub episode_id: i32,
    #[sea_orm(not_null)]
    pub playback_status: PlaybackStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::user::Entity",
        from = "Column::UserId",
        to = "super::user::Column::Id"
    )]
    User,
    #[sea_orm(
        belongs_to = "super::episode::Entity",
        from = "Column::EpisodeId",
        to = "super::episode::Column::Id"
    )]
    Episode,
}

impl Related<super::user::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::User.def()
    }
}

impl Related<super::episode::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Episode.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
