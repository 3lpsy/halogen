use super::common::TableWithTimestamps;
use super::m20250517_210000_create_podcasts_table::Podcast;
use sea_orm_migration::prelude::*;

/// `user_podcast` — subscription junction: which podcasts a user subscribes to.
/// Composite PK `(user_id, podcast_id)` doubles as the uniqueness guard. FKs are
/// `ON DELETE CASCADE` and enforced (`foreign_keys` is ON); delete handlers also
/// clean up explicitly. The `updated_at` trigger is installed by the later
/// `m20260616_000200_add_updated_at_triggers` (keyed by `rowid`, since this table
/// has no surrogate `id`).
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let mut fk_user = ForeignKey::create();
        fk_user
            .name("fk-user_podcast-user")
            .from_tbl(UserPodcast::Table)
            .from_col(UserPodcast::UserId)
            .to_tbl(User::Table)
            .to_col(User::Id)
            .on_delete(ForeignKeyAction::Cascade);

        let mut fk_podcast = ForeignKey::create();
        fk_podcast
            .name("fk-user_podcast-podcast")
            .from_tbl(UserPodcast::Table)
            .from_col(UserPodcast::PodcastId)
            .to_tbl(Podcast::Table)
            .to_col(Podcast::Id)
            .on_delete(ForeignKeyAction::Cascade);

        manager
            .create_table(
                Table::create()
                    .table(UserPodcast::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(UserPodcast::UserId).integer().not_null())
                    .col(ColumnDef::new(UserPodcast::PodcastId).integer().not_null())
                    .primary_key(
                        Index::create()
                            .col(UserPodcast::UserId)
                            .col(UserPodcast::PodcastId),
                    )
                    .add_timestamps()
                    .foreign_key(&mut fk_user)
                    .foreign_key(&mut fk_podcast)
                    .to_owned(),
            )
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(UserPodcast::Table).to_owned())
            .await
    }
}

#[derive(Iden)]
pub enum UserPodcast {
    Table,
    UserId,
    PodcastId,
}

#[derive(Iden)]
#[allow(dead_code)]
enum User {
    Table,
    Id,
}
