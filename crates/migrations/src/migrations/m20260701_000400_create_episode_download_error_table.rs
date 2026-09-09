//! Episode download-failure history: one row per failed media download with the
//! failure reason, FK'd to the episode (errors die with their episode).

use super::common::{MigrationTimestampExt, TableWithTimestamps};
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let mut episode_fk = ForeignKey::create();
        episode_fk
            .name("fk-episode_download_error-episode")
            .from_tbl(EpisodeDownloadError::Table)
            .from_col(EpisodeDownloadError::EpisodeId)
            .to_tbl(Episode::Table)
            .to_col(Episode::Id)
            .on_delete(ForeignKeyAction::Cascade);

        manager
            .create_table(
                Table::create()
                    .table(EpisodeDownloadError::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(EpisodeDownloadError::Id)
                            .integer()
                            .auto_increment()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(EpisodeDownloadError::EpisodeId)
                            .integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(EpisodeDownloadError::Reason)
                            .text()
                            .not_null(),
                    )
                    .add_timestamps()
                    .foreign_key(&mut episode_fk)
                    .to_owned(),
            )
            .await?;

        // Read newest-per-episode (the errors page) and pruned per episode.
        manager
            .create_index(
                Index::create()
                    .name("idx-episode_download_error-episode")
                    .table(EpisodeDownloadError::Table)
                    .col(EpisodeDownloadError::EpisodeId)
                    .to_owned(),
            )
            .await?;

        self.create_timestamp_trigger(manager, EpisodeDownloadError::Table.to_string())
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        self.drop_timestamp_trigger(manager, EpisodeDownloadError::Table.to_string())
            .await?;
        manager
            .drop_index(
                Index::drop()
                    .name("idx-episode_download_error-episode")
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(Table::drop().table(EpisodeDownloadError::Table).to_owned())
            .await
    }
}

#[derive(Iden)]
#[allow(dead_code)]
enum EpisodeDownloadError {
    Table,
    Id,
    EpisodeId,
    Reason,
}

#[derive(Iden)]
#[allow(dead_code)]
enum Episode {
    Table,
    Id,
}
