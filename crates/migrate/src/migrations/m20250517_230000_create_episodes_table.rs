use halogen_wire::enums::DownloadStatus;

use super::common::{MigrationTimestampExt, TableWithTimestamps};
use super::m20250517_210000_create_podcasts_table::Podcast;
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let mut fk = ForeignKey::create();
        fk.name("fk-episodes-podcast")
            .from_tbl(Episode::Table)
            .from_col(Episode::PodcastId)
            .to_tbl(Podcast::Table)
            .to_col(Podcast::Id)
            .on_delete(ForeignKeyAction::Cascade);

        manager
            .create_table(
                Table::create()
                    .table(Episode::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Episode::Id)
                            .integer()
                            .auto_increment()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(Episode::PodcastId).integer().not_null())
                    .col(ColumnDef::new(Episode::Title).string().not_null())
                    .col(ColumnDef::new(Episode::Description).text().not_null())
                    .col(ColumnDef::new(Episode::ContentUrl).string().not_null())
                    .col(ColumnDef::new(Episode::ArtUrl).string())
                    .col(ColumnDef::new(Episode::PublishedAt).timestamp())
                    .col(ColumnDef::new(Episode::DownloadedAt).timestamp())
                    .col(ColumnDef::new(Episode::ContentFilePath).string())
                    .col(ColumnDef::new(Episode::DownloadSize).big_integer())
                    .col(ColumnDef::new(Episode::ArtFilePath).string())
                    .col(
                        ColumnDef::new(Episode::DownloadStatus)
                            .string()
                            .not_null()
                            .default(DownloadStatus::NotDownloaded),
                    )
                    // Stable RSS <guid>, preferred over content_url for identity.
                    .col(ColumnDef::new(Episode::Guid).string())
                    .col(ColumnDef::new(Episode::DurationSecs).integer())
                    .add_timestamps()
                    .foreign_key(&mut fk)
                    .to_owned(),
            )
            .await?;
        self.create_timestamp_trigger(manager, Episode::Table.to_string())
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        self.drop_timestamp_trigger(manager, Episode::Table.to_string())
            .await?;
        manager
            .drop_table(Table::drop().table(Episode::Table).to_owned())
            .await
    }
}

#[derive(Iden)]
#[allow(dead_code)]
#[allow(clippy::enum_variant_names)]
enum Episode {
    Table,
    Id,
    PodcastId,
    Title,
    Description,
    ContentUrl,
    ArtUrl,
    PublishedAt,
    DownloadedAt,
    ContentFilePath,
    DownloadSize,
    ArtFilePath,
    DownloadStatus,
    Guid,
    DurationSecs,
    CreatedAt,
}
