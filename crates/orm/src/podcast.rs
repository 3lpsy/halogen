use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;

use crate::podcast_config;
use crate::user;

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "podcast_config::Entity",
        from = "Column::PodcastConfigId",
        to = "podcast_config::Column::Id"
    )]
    PodcastConfig,
    #[sea_orm(
        belongs_to = "user::Entity",
        from = "Column::OwnerId",
        to = "user::Column::Id"
    )]
    Owner,
}

impl Related<podcast_config::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::PodcastConfig.def()
    }
}

impl Related<user::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Owner.def()
    }
}

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "podcast")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = true)]
    pub id: i32,
    pub title: String,
    pub description: String,
    pub feed_url: String,
    pub art_url: Option<String>,
    pub author: Option<String>,
    pub polled_at: Option<DateTime<Utc>>,
    pub podcast_config_id: Option<i32>,
    /// The user who first added this podcast. Server-derived from the auth token
    /// on create; never accepted from the client (store/update DTOs omit it).
    pub owner_id: i32,
    pub art_file_path: Option<String>,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    /// Redirect hop chain from the last successful poll, comma-joined and starting
    /// with `feed_url` (so a direct feed equals `feed_url`). NULL until first poll.
    pub feed_url_redirects: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl ActiveModelBehavior for ActiveModel {}

/// Sort keys accepted by the podcast list endpoints (unknown → primary key).
impl crate::common::Sortable for Entity {
    fn order_column(order_by: &str) -> Column {
        match order_by {
            "title" => Column::Title,
            "polled_at" => Column::PolledAt,
            "created_at" => Column::CreatedAt,
            "updated_at" => Column::UpdatedAt,
            _ => Column::Id,
        }
    }
}
