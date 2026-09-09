//! Durable poll-job history (one row per sync run — manual, on-demand, or the
//! scheduled service tick). Replaces the in-memory `JobTracker` storage so sync
//! status survives restarts and the status endpoints read the DB.

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
                    .table(PollJob::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(PollJob::Id)
                            .integer()
                            .auto_increment()
                            .not_null()
                            .primary_key(),
                    )
                    // running | completed | failed (wire `PollJobStatus`).
                    .col(ColumnDef::new(PollJob::Status).string().not_null())
                    // manual | scheduled (wire `PollJobTrigger`).
                    .col(ColumnDef::new(PollJob::Trigger).string().not_null())
                    // Scope of a single-podcast job (`None` = all feeds). A plain
                    // snapshot column, NOT an FK: the job history must survive the
                    // podcast being deleted.
                    .col(ColumnDef::new(PollJob::PodcastId).integer())
                    .col(ColumnDef::new(PollJob::StartedAt).timestamp().not_null())
                    .col(ColumnDef::new(PollJob::CompletedAt).timestamp())
                    .col(
                        ColumnDef::new(PollJob::TotalNew)
                            .integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(PollJob::TotalUpdated)
                            .integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(PollJob::TotalErrors)
                            .integer()
                            .not_null()
                            .default(0),
                    )
                    .add_timestamps()
                    .to_owned(),
            )
            .await?;
        self.create_timestamp_trigger(manager, PollJob::Table.to_string())
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        self.drop_timestamp_trigger(manager, PollJob::Table.to_string())
            .await?;
        manager
            .drop_table(Table::drop().table(PollJob::Table).to_owned())
            .await
    }
}

#[derive(Iden)]
#[allow(dead_code)]
pub enum PollJob {
    Table,
    Id,
    Status,
    Trigger,
    PodcastId,
    StartedAt,
    CompletedAt,
    TotalNew,
    TotalUpdated,
    TotalErrors,
}
