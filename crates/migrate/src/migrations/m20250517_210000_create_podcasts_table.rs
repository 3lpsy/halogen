use super::common::{MigrationTimestampExt, TableWithTimestamps};
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let mut owner_fk = ForeignKey::create();
        owner_fk
            .name("fk-podcast-owner")
            .from_tbl(Podcast::Table)
            .from_col(Podcast::OwnerId)
            .to_tbl(User::Table)
            .to_col(User::Id)
            .on_delete(ForeignKeyAction::Cascade);

        // `podcast_config_id` optional, 1:1. SET NULL: deleting the config row leaves
        // the podcast configless. Parent `podcast_config` is created first (see mod.rs).
        let mut config_fk = ForeignKey::create();
        config_fk
            .name("fk-podcast-config")
            .from_tbl(Podcast::Table)
            .from_col(Podcast::PodcastConfigId)
            .to_tbl(PodcastConfig::Table)
            .to_col(PodcastConfig::Id)
            .on_delete(ForeignKeyAction::SetNull);

        manager
            .create_table(
                Table::create()
                    .table(Podcast::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Podcast::Id)
                            .integer()
                            .auto_increment()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(Podcast::Title).string().not_null())
                    .col(ColumnDef::new(Podcast::Description).text())
                    .col(
                        ColumnDef::new(Podcast::FeedUrl)
                            .string()
                            .not_null()
                            .unique_key(),
                    )
                    .col(ColumnDef::new(Podcast::ArtUrl).string())
                    .col(ColumnDef::new(Podcast::Author).string())
                    .col(ColumnDef::new(Podcast::PodcastConfigId).integer())
                    .col(ColumnDef::new(Podcast::ArtFilePath).string())
                    .col(ColumnDef::new(Podcast::Etag).string())
                    .col(ColumnDef::new(Podcast::LastModified).string())
                    .col(ColumnDef::new(Podcast::PolledAt).date_time().null())
                    // Owner: user who first added this podcast. FK → user(id) CASCADE.
                    .col(ColumnDef::new(Podcast::OwnerId).integer().not_null())
                    .add_timestamps()
                    .foreign_key(&mut owner_fk)
                    .foreign_key(&mut config_fk)
                    .to_owned(),
            )
            .await?;
        self.create_timestamp_trigger(manager, Podcast::Table.to_string())
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        self.drop_timestamp_trigger(manager, Podcast::Table.to_string())
            .await?;
        manager
            .drop_table(Table::drop().table(Podcast::Table).to_owned())
            .await
    }
}

#[derive(Iden)]
pub enum Podcast {
    Table,
    Id,
    Title,
    Description,
    FeedUrl,
    ArtUrl,
    Author,
    PodcastConfigId,
    ArtFilePath,
    Etag,
    LastModified,
    PolledAt,
    OwnerId,
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
enum PodcastConfig {
    Table,
    Id,
}
