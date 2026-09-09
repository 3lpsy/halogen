use sea_orm_migration::prelude::*;
#[derive(DeriveMigrationName)]
pub struct Migration;
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.get_connection().execute_unprepared("CREATE TABLE sync_epoch (id INTEGER PRIMARY KEY CHECK(id = 1), epoch TEXT NOT NULL, floor_sequence INTEGER NOT NULL DEFAULT 0); INSERT INTO sync_epoch(id, epoch) VALUES (1, lower(hex(randomblob(16))))").await?;
        Ok(())
    }
    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared("DROP TABLE sync_epoch")
            .await?;
        Ok(())
    }
}
