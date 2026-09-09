use sea_orm_migration::prelude::*;

/// Add `playlist.on_remove_delete_file_server` + `playlist.on_remove_delete_file_client`
/// (non-null BOOLEAN, default FALSE): per-playlist cleanup flags. When an episode
/// is removed from a flagged playlist, the server flag deletes the shared
/// server-side download file (only once the episode belongs to no other playlist)
/// and the client flag tells the removing device to drop its local media copy.
/// The create migration is left untouched; fresh installs pick the columns up here.
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        // SQLite allows only one column per ALTER TABLE.
        db.execute_unprepared(
            "ALTER TABLE playlist ADD COLUMN on_remove_delete_file_server BOOLEAN NOT NULL DEFAULT FALSE",
        )
        .await?;
        db.execute_unprepared(
            "ALTER TABLE playlist ADD COLUMN on_remove_delete_file_client BOOLEAN NOT NULL DEFAULT FALSE",
        )
        .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared("ALTER TABLE playlist DROP COLUMN on_remove_delete_file_server")
            .await?;
        db.execute_unprepared("ALTER TABLE playlist DROP COLUMN on_remove_delete_file_client")
            .await?;
        Ok(())
    }
}
