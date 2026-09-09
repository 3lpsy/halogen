use sea_orm_migration::prelude::*;

/// Add a `feed_url_redirects` column to `podcast`: the redirect hop chain seen
/// the last time the feed was polled, as a comma-joined CSV starting with the
/// stored `feed_url`. A direct feed stores exactly `feed_url` (single element),
/// so a redirect is detectable with `feed_url_redirects != feed_url`. NULL until
/// the first successful poll. Fresh installs get the column here too (the create
/// migration is left untouched).
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared("ALTER TABLE podcast ADD COLUMN feed_url_redirects TEXT")
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared("ALTER TABLE podcast DROP COLUMN feed_url_redirects")
            .await?;
        Ok(())
    }
}
