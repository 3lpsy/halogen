use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq, Serialize, Deserialize)]
#[sea_orm(table_name = "playlist")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = true)]
    pub id: i32,
    pub name: String,
    pub description: Option<String>,
    /// Owner: every playlist belongs to exactly one user. Server-derived from the
    /// auth token; never accepted from the client (store/update DTOs omit it).
    pub user_id: i32,
    pub is_default: bool,
    /// Manual order within the user's playlists (0-based). Mirrors
    /// `episode_playlist.position`; drives the default "Custom" sort and the
    /// stable ordering the paged list uses.
    pub position: i32,
    /// When an episode is removed from this playlist, delete its server-side
    /// download file — only once the episode belongs to no other playlist (the
    /// file is one global per-episode copy shared across users).
    pub on_remove_delete_file_server: bool,
    /// When the client removes an episode from this playlist, it also deletes
    /// its on-device media copy. Stored server-side; behavior lives in the client.
    pub on_remove_delete_file_client: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "super::episode_playlist::Entity")]
    EpisodePlaylist,
    #[sea_orm(
        belongs_to = "super::user::Entity",
        from = "Column::UserId",
        to = "super::user::Column::Id"
    )]
    Owner,
}

impl Related<super::episode_playlist::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::EpisodePlaylist.def()
    }
}

impl Related<super::user::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Owner.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}

/// Sort keys accepted by the playlist list endpoints (unknown → primary key).
impl crate::common::Sortable for Entity {
    fn order_column(order_by: &str) -> Column {
        match order_by {
            "name" => Column::Name,
            "position" => Column::Position,
            "created_at" => Column::CreatedAt,
            "updated_at" => Column::UpdatedAt,
            _ => Column::Id,
        }
    }
}
