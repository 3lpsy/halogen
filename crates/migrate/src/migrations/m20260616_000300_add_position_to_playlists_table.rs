use sea_orm_migration::prelude::*;

/// Add a manual `position` column to `playlist` so playlists carry their own
/// order (mirroring `episode_playlist.position`), used as the default "Custom"
/// sort and as the stable key the lazy/paged playlist list orders by.
///
/// Accounts for existing databases: after adding the column (default 0), backfill
/// a per-user 0-based order by id so each user's current playlists get distinct
/// positions. Fresh installs get the column here too (the create migration is
/// left untouched).
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        db.execute_unprepared(
            "ALTER TABLE playlist ADD COLUMN position INTEGER NOT NULL DEFAULT 0",
        )
        .await?;

        // Backfill: per user, assign 0-based positions by ascending id so existing
        // playlists get a deterministic, distinct order.
        db.execute_unprepared(
            "UPDATE playlist SET position = (\
                 SELECT COUNT(*) FROM playlist p2 \
                 WHERE p2.user_id = playlist.user_id AND p2.id < playlist.id\
             )",
        )
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared("ALTER TABLE playlist DROP COLUMN position")
            .await?;
        Ok(())
    }
}
