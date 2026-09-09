use super::common::MigrationTimestampExt;
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

/// Backfill `updated_at` triggers for the tables created without one
/// (`playlist`, `episode_playlist`, `podcast_auto_playlist`, `user_podcast`) so
/// every table keeps `updated_at` fresh on write, like the rest of the schema.
/// These match the row by `rowid` (they have a composite PK or no surrogate `id`).
const TABLES: [&str; 4] = [
    "playlist",
    "episode_playlist",
    "podcast_auto_playlist",
    "user_podcast",
];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for table in TABLES {
            self.create_rowid_timestamp_trigger(manager, table.to_string())
                .await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for table in TABLES {
            self.drop_timestamp_trigger(manager, table.to_string())
                .await?;
        }
        Ok(())
    }
}
