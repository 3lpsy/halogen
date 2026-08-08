use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

/// Subscription junction: a user is subscribed to a podcast. Composite PK
/// `(user_id, podcast_id)`.
#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq, Serialize, Deserialize)]
#[sea_orm(table_name = "user_podcast")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub user_id: i32,
    #[sea_orm(primary_key)]
    pub podcast_id: i32,
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
        belongs_to = "super::podcast::Entity",
        from = "Column::PodcastId",
        to = "super::podcast::Column::Id"
    )]
    Podcast,
}

impl Related<super::user::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::User.def()
    }
}

impl Related<super::podcast::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Podcast.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
