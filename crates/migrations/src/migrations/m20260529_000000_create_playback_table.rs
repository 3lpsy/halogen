use super::common::{MigrationTimestampExt, TableWithTimestamps};
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let mut user_fk = ForeignKey::create();
        user_fk
            .name("fk-playback-user")
            .from_tbl(Playback::Table)
            .from_col(Playback::UserId)
            .to_tbl(User::Table)
            .to_col(User::Id)
            .on_delete(ForeignKeyAction::Cascade);

        let mut episode_fk = ForeignKey::create();
        episode_fk
            .name("fk-playback-episode")
            .from_tbl(Playback::Table)
            .from_col(Playback::EpisodeId)
            .to_tbl(Episode::Table)
            .to_col(Episode::Id)
            .on_delete(ForeignKeyAction::Cascade);

        manager
            .create_table(
                Table::create()
                    .table(Playback::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Playback::Id)
                            .integer()
                            .auto_increment()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(Playback::UserId).integer().not_null())
                    .col(ColumnDef::new(Playback::EpisodeId).integer().not_null())
                    .col(ColumnDef::new(Playback::Cursor).integer().not_null())
                    .col(
                        ColumnDef::new(Playback::Completed)
                            .boolean()
                            .not_null()
                            .default(false),
                    )
                    .add_timestamps()
                    .foreign_key(&mut user_fk)
                    .foreign_key(&mut episode_fk)
                    .to_owned(),
            )
            .await?;

        // Unique index backing the playback upsert (one row per user+episode).
        manager
            .create_index(
                Index::create()
                    .name("idx-playback-unique")
                    .table(Playback::Table)
                    .col(Playback::UserId)
                    .col(Playback::EpisodeId)
                    .unique()
                    .to_owned(),
            )
            .await?;

        self.create_timestamp_trigger(manager, Playback::Table.to_string())
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        self.drop_timestamp_trigger(manager, Playback::Table.to_string())
            .await?;
        manager
            .drop_index(Index::drop().name("idx-playback-unique").to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(Playback::Table).to_owned())
            .await
    }
}

#[derive(Iden)]
#[allow(dead_code)]
enum Playback {
    Table,
    Id,
    UserId,
    EpisodeId,
    Cursor,
    Completed,
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
