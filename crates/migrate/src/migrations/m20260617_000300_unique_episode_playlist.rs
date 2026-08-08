use sea_orm_migration::prelude::*;

/// Enforce one membership per (episode, playlist) on `episode_playlist`. The
/// create migration declared no primary key or unique index, so older builds could
/// insert the same episode into a playlist multiple times (the sea-orm entity
/// claims a composite PK the DB never enforced). Deduplicate existing rows — keep
/// the lowest rowid per pair — then add the unique index.
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        // Drop duplicate memberships before the unique index can be created.
        db.execute_unprepared(
            "DELETE FROM episode_playlist \
             WHERE rowid NOT IN ( \
                 SELECT MIN(rowid) FROM episode_playlist \
                 GROUP BY episode_id, playlist_id \
             )",
        )
        .await?;

        db.execute_unprepared(
            "CREATE UNIQUE INDEX IF NOT EXISTS \"idx-episode_playlist-episode-playlist\" \
             ON episode_playlist (episode_id, playlist_id)",
        )
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared("DROP INDEX IF EXISTS \"idx-episode_playlist-episode-playlist\"")
            .await?;
        Ok(())
    }
}
