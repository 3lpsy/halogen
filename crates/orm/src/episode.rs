use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;

use crate::podcast;
use halogen_wire::DownloadStatus;

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::podcast::Entity",
        from = "Column::PodcastId",
        to = "super::podcast::Column::Id"
    )]
    Podcast,
}

impl Related<podcast::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Podcast.def()
    }
}

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "episode")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = true)]
    pub id: i32,
    #[sea_orm(not_null)]
    pub podcast_id: i32,
    #[sea_orm(not_null)]
    pub title: String,
    #[sea_orm(column_type = "Text")]
    pub description: String,
    #[sea_orm(not_null)]
    pub content_url: String,
    /// Stable RSS `<guid>`, preferred over `content_url` for episode identity.
    /// Nullable: legacy rows and feeds without a guid fall back to `content_url`.
    pub guid: Option<String>,
    pub art_url: Option<String>,
    pub published_at: Option<DateTime<Utc>>,
    pub downloaded_at: Option<DateTime<Utc>>,
    pub content_file_path: Option<String>,
    /// Size in bytes of the server-side downloaded file. `Some` only while a
    /// download exists; cleared (with `downloaded_at`/`content_file_path`) on removal.
    pub download_size: Option<i64>,
    pub art_file_path: Option<String>,
    #[sea_orm(not_null)]
    pub download_status: DownloadStatus,
    /// When the current/last download attempt flipped this row to `Downloading`.
    /// Reset by the stuck-download watchdog once older than the configured cutoff.
    pub download_started_at: Option<DateTime<Utc>>,
    /// Count of `download_episode` attempts; the recovery loop gives up
    /// (→ `DownloadBroken`) once this reaches the configured max.
    #[sea_orm(not_null)]
    pub download_attempts: i32,
    pub duration_secs: Option<i32>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl ActiveModelBehavior for ActiveModel {}

/// Sort keys accepted by the episode list endpoints (unknown → primary key).
impl crate::common::Sortable for Entity {
    fn order_column(order_by: &str) -> Column {
        match order_by {
            "title" => Column::Title,
            "published_at" => Column::PublishedAt,
            "created_at" => Column::CreatedAt,
            "updated_at" => Column::UpdatedAt,
            "duration_secs" => Column::DurationSecs,
            _ => Column::Id,
        }
    }
}
