use super::common::{MigrationTimestampExt, TableWithTimestamps};
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let mut episode_fk = ForeignKey::create();
        episode_fk
            .name("fk-episode_chapter-episode")
            .from_tbl(EpisodeChapter::Table)
            .from_col(EpisodeChapter::EpisodeId)
            .to_tbl(Episode::Table)
            .to_col(Episode::Id)
            .on_delete(ForeignKeyAction::Cascade);

        manager
            .create_table(
                Table::create()
                    .table(EpisodeChapter::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(EpisodeChapter::Id)
                            .integer()
                            .auto_increment()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(EpisodeChapter::EpisodeId)
                            .integer()
                            .not_null(),
                    )
                    .col(ColumnDef::new(EpisodeChapter::Title).string().not_null())
                    .col(
                        ColumnDef::new(EpisodeChapter::StartsAtSecs)
                            .integer()
                            .not_null(),
                    )
                    .add_timestamps()
                    .foreign_key(&mut episode_fk)
                    .to_owned(),
            )
            .await?;

        // Chapters are always loaded by episode (batched include); index the FK.
        manager
            .create_index(
                Index::create()
                    .name("idx-episode_chapter-episode")
                    .table(EpisodeChapter::Table)
                    .col(EpisodeChapter::EpisodeId)
                    .to_owned(),
            )
            .await?;

        self.create_timestamp_trigger(manager, EpisodeChapter::Table.to_string())
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        self.drop_timestamp_trigger(manager, EpisodeChapter::Table.to_string())
            .await?;
        manager
            .drop_index(Index::drop().name("idx-episode_chapter-episode").to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(EpisodeChapter::Table).to_owned())
            .await
    }
}

#[derive(Iden)]
#[allow(dead_code)]
enum EpisodeChapter {
    Table,
    Id,
    EpisodeId,
    Title,
    StartsAtSecs,
    CreatedAt,
}

#[derive(Iden)]
#[allow(dead_code)]
enum Episode {
    Table,
    Id,
}
