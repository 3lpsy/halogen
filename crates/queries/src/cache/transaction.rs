use super::database::CacheDatabase;
use anyhow::Result;
use halogen_sync_enrich::{OutboxOp, StoreChanges};
use rusqlite::{OptionalExtension, params};

impl CacheDatabase {
    pub(crate) fn commit_transaction(
        &self,
        changes: &StoreChanges,
        operations: &[OutboxOp],
    ) -> Result<()> {
        let mut connection = self.conn.borrow_mut();
        let transaction = connection.transaction()?;
        if changes.check_sync_cursor {
            let cursor: Option<String> = transaction
                .query_row(
                    "SELECT value FROM sync_metadata WHERE key = 'cursor'",
                    [],
                    |row| row.get(0),
                )
                .optional()?;
            anyhow::ensure!(
                cursor == changes.expected_sync_cursor,
                "sync cursor advanced during pull"
            );
        }
        if changes.require_empty_pending {
            let mut query = transaction.prepare("SELECT op FROM outbox")?;
            let rows = query.query_map([], |row| row.get::<_, String>(0))?;
            for row in rows {
                let entry = halogen_sync_enrich::JournalEntry::from(serde_json::from_str::<
                    halogen_sync_enrich::StoredEntry,
                >(&row?)?);
                anyhow::ensure!(
                    entry.rejection.is_some() || entry.delivered,
                    "pending changes prevent cache replacement"
                );
            }
        }
        if changes.reset_cache {
            for table in [
                "podcasts",
                "episodes",
                "playlists",
                "playbacks",
                "podcast_auto_playlists",
                "sync_metadata",
            ] {
                transaction.execute(&format!("DELETE FROM {table}"), [])?;
            }
        }
        for id in &changes.deleted_auto_playlists {
            transaction.execute(
                "DELETE FROM podcast_auto_playlists WHERE podcast_id = ?1",
                [id],
            )?;
        }
        for (id, rows) in &changes.auto_playlists {
            transaction.execute(
                "INSERT OR REPLACE INTO podcast_auto_playlists (podcast_id,data) VALUES (?1,?2)",
                params![id, serde_json::to_string(rows)?],
            )?;
        }
        if let Some(cursor) = &changes.sync_cursor {
            anyhow::ensure!(
                !cursor.is_empty() && cursor.len() <= 80,
                "invalid sync cursor"
            );
            transaction.execute(
                "INSERT OR REPLACE INTO sync_metadata (key,value) VALUES ('cursor',?1)",
                [cursor],
            )?;
        }
        for (table, key, ids) in [
            ("podcasts", "id", &changes.deleted_podcasts),
            ("episodes", "id", &changes.deleted_episodes),
            ("playlists", "id", &changes.deleted_playlists),
            ("playbacks", "episode_id", &changes.deleted_playbacks),
        ] {
            for id in ids {
                anyhow::ensure!(*id > 0, "invalid cached row id");
                transaction.execute(&format!("DELETE FROM {table} WHERE {key} = ?1"), [id])?;
            }
        }
        for row in &changes.podcasts {
            transaction.execute(
                "INSERT OR REPLACE INTO podcasts (id, data) VALUES (?1, ?2)",
                params![row.id, serde_json::to_string(row)?],
            )?;
        }
        for row in &changes.episodes {
            transaction.execute("INSERT OR REPLACE INTO episodes (id, podcast_id, data, published_at) VALUES (?1, ?2, ?3, ?4)",
                params![row.id, row.podcast_id, serde_json::to_string(row)?, row.published_at.map(|at| at.timestamp())])?;
        }
        for row in &changes.playlists {
            transaction.execute(
                "INSERT OR REPLACE INTO playlists (id, data) VALUES (?1, ?2)",
                params![row.id, serde_json::to_string(row)?],
            )?;
        }
        for row in &changes.playbacks {
            transaction.execute(
                "INSERT OR REPLACE INTO playbacks (episode_id, data) VALUES (?1, ?2)",
                params![row.episode_id, serde_json::to_string(row)?],
            )?;
        }
        for id in &changes.acknowledged_operations {
            let id = i64::try_from(*id)?;
            anyhow::ensure!(id > 0, "invalid journal id");
            transaction.execute("DELETE FROM outbox WHERE id = ?1", [id])?;
        }
        for operation in operations {
            transaction.execute(
                "INSERT INTO outbox (op) VALUES (?1)",
                [serde_json::to_string(operation)?],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }
}
