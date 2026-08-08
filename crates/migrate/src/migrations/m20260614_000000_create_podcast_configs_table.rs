use super::common::{MigrationTimestampExt, TableWithTimestamps};
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(PodcastConfig::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(PodcastConfig::Id)
                            .integer()
                            .auto_increment()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(PodcastConfig::PollIntervalSeconds).integer())
                    .col(ColumnDef::new(PodcastConfig::MaxEpisodes).integer())
                    .col(ColumnDef::new(PodcastConfig::MaxConcurrentDownloads).integer())
                    .col(ColumnDef::new(PodcastConfig::AutoDownloadEnabled).boolean())
                    .add_timestamps()
                    .to_owned(),
            )
            .await?;
        self.create_timestamp_trigger(manager, PodcastConfig::Table.to_string())
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        self.drop_timestamp_trigger(manager, PodcastConfig::Table.to_string())
            .await?;
        manager
            .drop_table(Table::drop().table(PodcastConfig::Table).to_owned())
            .await
    }
}

#[derive(Iden)]
pub enum PodcastConfig {
    Table,
    Id,
    PollIntervalSeconds,
    MaxEpisodes,
    MaxConcurrentDownloads,
    AutoDownloadEnabled,
    CreatedAt,
}
