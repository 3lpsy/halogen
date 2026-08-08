//! Podcast RSS sync-failure history: one row per failed fetch/parse with the
//! failure reason, FK'd to the podcast (errors die with their podcast).

use super::common::{MigrationTimestampExt, TableWithTimestamps};
use super::m20250517_210000_create_podcasts_table::Podcast;
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let mut podcast_fk = ForeignKey::create();
        podcast_fk
            .name("fk-podcast_sync_error-podcast")
            .from_tbl(PodcastSyncError::Table)
            .from_col(PodcastSyncError::PodcastId)
            .to_tbl(Podcast::Table)
            .to_col(Podcast::Id)
            .on_delete(ForeignKeyAction::Cascade);

        manager
            .create_table(
                Table::create()
                    .table(PodcastSyncError::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(PodcastSyncError::Id)
                            .integer()
                            .auto_increment()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(PodcastSyncError::PodcastId)
                            .integer()
                            .not_null(),
                    )
                    .col(ColumnDef::new(PodcastSyncError::Reason).text().not_null())
                    .add_timestamps()
                    .foreign_key(&mut podcast_fk)
                    .to_owned(),
            )
            .await?;

        // Read newest-per-podcast (the errors page) and pruned per podcast.
        manager
            .create_index(
                Index::create()
                    .name("idx-podcast_sync_error-podcast")
                    .table(PodcastSyncError::Table)
                    .col(PodcastSyncError::PodcastId)
                    .to_owned(),
            )
            .await?;

        self.create_timestamp_trigger(manager, PodcastSyncError::Table.to_string())
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        self.drop_timestamp_trigger(manager, PodcastSyncError::Table.to_string())
            .await?;
        manager
            .drop_index(
                Index::drop()
                    .name("idx-podcast_sync_error-podcast")
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(Table::drop().table(PodcastSyncError::Table).to_owned())
            .await
    }
}

#[derive(Iden)]
#[allow(dead_code)]
enum PodcastSyncError {
    Table,
    Id,
    PodcastId,
    Reason,
}
