use sea_orm_migration::prelude::*;

/// Add `podcast_auto_playlist.add_to_start` (nullable BOOLEAN): the per-link
/// insert-position override for auto-added episodes. NULL (all existing rows)
/// means "follow the server-wide `subscription_auto_playlist_add_to_start`
/// default"; TRUE inserts new episodes at the start of the playlist, FALSE
/// appends at the end. The create migration is left untouched; fresh installs
/// pick the column up here too.
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared("ALTER TABLE podcast_auto_playlist ADD COLUMN add_to_start BOOLEAN")
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared("ALTER TABLE podcast_auto_playlist DROP COLUMN add_to_start")
            .await?;
        Ok(())
    }
}
