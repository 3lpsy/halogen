use super::common::{MigrationTimestampExt, TableWithTimestamps};
use sea_orm_migration::prelude::*;

/// `user_episode_status` — per-user listen state (Unplayed/Played/Finished),
/// broken out of the old global `episode.playback_status` column. One row per
/// `(user, episode)` (unique index below), maintained by the playback upsert
/// handler. FKs are `ON DELETE CASCADE` and enforced (`foreign_keys` is ON);
/// delete handlers also clean up explicitly.
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let mut user_fk = ForeignKey::create();
        user_fk
            .name("fk-user_episode_status-user")
            .from_tbl(UserEpisodeStatus::Table)
            .from_col(UserEpisodeStatus::UserId)
            .to_tbl(User::Table)
            .to_col(User::Id)
            .on_delete(ForeignKeyAction::Cascade);

        let mut episode_fk = ForeignKey::create();
        episode_fk
            .name("fk-user_episode_status-episode")
            .from_tbl(UserEpisodeStatus::Table)
            .from_col(UserEpisodeStatus::EpisodeId)
            .to_tbl(Episode::Table)
            .to_col(Episode::Id)
            .on_delete(ForeignKeyAction::Cascade);

        manager
            .create_table(
                Table::create()
                    .table(UserEpisodeStatus::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(UserEpisodeStatus::Id)
                            .integer()
                            .auto_increment()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(UserEpisodeStatus::UserId)
                            .integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(UserEpisodeStatus::EpisodeId)
                            .integer()
                            .not_null(),
                    )
                    // Per-user listen state; mirrors the old episode column default.
                    .col(
                        ColumnDef::new(UserEpisodeStatus::PlaybackStatus)
                            .string()
                            .not_null()
                            .default("UNPLAYED"),
                    )
                    .add_timestamps()
                    .foreign_key(&mut user_fk)
                    .foreign_key(&mut episode_fk)
                    .to_owned(),
            )
            .await?;

        // Unique index backing the per-user status upsert (one row per user+episode).
        manager
            .create_index(
                Index::create()
                    .name("idx-user_episode_status-unique")
                    .table(UserEpisodeStatus::Table)
                    .col(UserEpisodeStatus::UserId)
                    .col(UserEpisodeStatus::EpisodeId)
                    .unique()
                    .to_owned(),
            )
            .await?;

        self.create_timestamp_trigger(manager, UserEpisodeStatus::Table.to_string())
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        self.drop_timestamp_trigger(manager, UserEpisodeStatus::Table.to_string())
            .await?;
        manager
            .drop_index(
                Index::drop()
                    .name("idx-user_episode_status-unique")
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(Table::drop().table(UserEpisodeStatus::Table).to_owned())
            .await
    }
}

#[derive(Iden)]
#[allow(dead_code)]
enum UserEpisodeStatus {
    Table,
    Id,
    UserId,
    EpisodeId,
    PlaybackStatus,
    CreatedAt,
}

#[derive(Iden)]
#[allow(dead_code)]
enum User {
    Table,
    Id,
}

#[derive(Iden)]
#[allow(dead_code)]
enum Episode {
    Table,
    Id,
}
