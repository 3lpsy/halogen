use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

/// Junction `(podcast_id, playlist_id)`: the RSS poller appends every newly-
/// ingested episode of the podcast to each playlist linked here.
#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq, Serialize, Deserialize)]
#[sea_orm(table_name = "podcast_auto_playlist")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub podcast_id: i32,
    #[sea_orm(primary_key)]
    pub playlist_id: i32,
    /// Insert position for auto-added episodes: `Some(true)` = start of the
    /// playlist, `Some(false)` = end, `None` = the server-wide
    /// `subscription_auto_playlist_add_to_start` default.
    pub add_to_start: Option<bool>,
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
    #[sea_orm(
        belongs_to = "super::playlist::Entity",
        from = "Column::PlaylistId",
        to = "super::playlist::Column::Id"
    )]
    Playlist,
}

impl Related<super::podcast::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Podcast.def()
    }
}

impl Related<super::playlist::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Playlist.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
