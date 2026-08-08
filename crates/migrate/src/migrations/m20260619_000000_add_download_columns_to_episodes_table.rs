use sea_orm_migration::prelude::*;

/// Add download reliability bookkeeping to `episode`:
///
/// - `download_started_at` (nullable TIMESTAMP): when the current/last attempt
///   flipped the row to `Downloading`. The stuck-download watchdog resets rows
///   whose start is older than the configured cutoff (or NULL — pre-migration
///   rows that were already mid-download).
/// - `download_attempts` (NOT NULL, default 0): incremented once per
///   `download_episode` call; the recovery loop gives up (→ `DOWNLOAD_BROKEN`)
///   once it reaches the configured max.
///
/// Existing rows get `download_attempts = 0` and a NULL start (treated as
/// orphaned by the watchdog, which is correct for any row stuck `Downloading`).
/// The create migration is left untouched; fresh installs pick the columns up
/// here too.
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        db.execute_unprepared("ALTER TABLE episode ADD COLUMN download_started_at TIMESTAMP")
            .await?;
        db.execute_unprepared(
            "ALTER TABLE episode ADD COLUMN download_attempts INTEGER NOT NULL DEFAULT 0",
        )
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared("ALTER TABLE episode DROP COLUMN download_attempts")
            .await?;
        db.execute_unprepared("ALTER TABLE episode DROP COLUMN download_started_at")
            .await?;
        Ok(())
    }
}
