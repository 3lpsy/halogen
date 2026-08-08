//! Per-podcast outcome rows for a poll job (child of `poll_job`) — one row per
//! feed a run touched, mirroring the wire `PodcastPollResultData`.

use super::common::{MigrationTimestampExt, TableWithTimestamps};
use super::m20260701_000100_create_poll_job_table::PollJob;
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let mut job_fk = ForeignKey::create();
        job_fk
            .name("fk-poll_job_podcast-poll_job")
            .from_tbl(PollJobPodcast::Table)
            .from_col(PollJobPodcast::PollJobId)
            .to_tbl(PollJob::Table)
            .to_col(PollJob::Id)
            .on_delete(ForeignKeyAction::Cascade);

        manager
            .create_table(
                Table::create()
                    .table(PollJobPodcast::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(PollJobPodcast::Id)
                            .integer()
                            .auto_increment()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(PollJobPodcast::PollJobId)
                            .integer()
                            .not_null(),
                    )
                    // Snapshot columns, NOT an FK to podcast: the run history keeps
                    // its record (id + title) even after the podcast is deleted.
                    .col(
                        ColumnDef::new(PollJobPodcast::PodcastId)
                            .integer()
                            .not_null(),
                    )
                    .col(ColumnDef::new(PollJobPodcast::Title).string().not_null())
                    // polled | skipped | error (wire `PodcastPollOutcome`).
                    .col(ColumnDef::new(PollJobPodcast::Outcome).string().not_null())
                    .col(
                        ColumnDef::new(PollJobPodcast::NewEpisodes)
                            .integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(PollJobPodcast::UpdatedEpisodes)
                            .integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(PollJobPodcast::Errors)
                            .integer()
                            .not_null()
                            .default(0),
                    )
                    .add_timestamps()
                    .foreign_key(&mut job_fk)
                    .to_owned(),
            )
            .await?;

        // Outcomes are always loaded by job; index the FK.
        manager
            .create_index(
                Index::create()
                    .name("idx-poll_job_podcast-poll_job")
                    .table(PollJobPodcast::Table)
                    .col(PollJobPodcast::PollJobId)
                    .to_owned(),
            )
            .await?;

        self.create_timestamp_trigger(manager, PollJobPodcast::Table.to_string())
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        self.drop_timestamp_trigger(manager, PollJobPodcast::Table.to_string())
            .await?;
        manager
            .drop_index(
                Index::drop()
                    .name("idx-poll_job_podcast-poll_job")
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(Table::drop().table(PollJobPodcast::Table).to_owned())
            .await
    }
}

#[derive(Iden)]
#[allow(dead_code)]
enum PollJobPodcast {
    Table,
    Id,
    PollJobId,
    PodcastId,
    Title,
    Outcome,
    NewEpisodes,
    UpdatedEpisodes,
    Errors,
}
