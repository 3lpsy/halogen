//! Native `LocalStore` backed by a SQLite file. Records are stored as JSON blobs of the `halogen_wire` DTOs
//! (keyed by id), exactly like the web `localStorage` store. This keeps the store DTO-agnostic — it never has
//! to track the DTO field layout in SQL columns — so the schema can't drift out of sync with the data types.
//! Indexed columns are kept only where we query by them (`episodes.podcast_id` and `episodes.published_at`).

use anyhow::Result;
use halogen_wire::{EpisodeData, PlaybackData, PlaylistData, PodcastData};
use rusqlite::Connection;
use std::path::PathBuf;

use halogen_sync_enrich::{
    EpisodeOrder, EpisodeQuery, EpisodeQueryFilter, OutboxOp, filter_count, filter_sort_paginate,
};

/// Cache-shape version, stamped into SQLite's `user_version`. Bump when a cached DTO's JSON shape changes
/// incompatibly: on the next open the CACHE tables (podcasts/episodes/playbacks/playlists — all re-fetchable)
/// are cleared and re-pulled, while the durable outbox is preserved. The native mirror of the web store's
/// `SCHEMA_VERSION` Cache-drop semantics — without it, native had no reset path for stale-shaped rows at all.
const CACHE_SCHEMA_VERSION: i32 = 1;

/// Native LocalStore backed by a SQLite file (JSON-blob rows).
pub struct CacheDatabase {
    pub(crate) conn: std::cell::RefCell<Connection>,
}

impl CacheDatabase {
    /// Open or create a LocalStore database at the given path, creating tables.
    pub fn open(path: PathBuf) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        let store = Self {
            conn: std::cell::RefCell::new(conn),
        };
        store.create_tables()?;
        store
            .conn
            .borrow()
            .execute_batch(include_str!("migrations/m0001_import_receipts.sql"))?;
        store
            .conn
            .borrow()
            .execute_batch(include_str!("migrations/m0002_sync_metadata.sql"))?;
        store.reset_stale_cache()?;
        Ok(store)
    }

    /// Import each legacy operation once, committing its receipt with its queue row.
    pub fn import_operations(&self, operations: &[(String, OutboxOp)]) -> Result<()> {
        let mut conn = self.conn.borrow_mut();
        let transaction = conn.transaction()?;
        for (id, operation) in operations {
            anyhow::ensure!(
                !id.is_empty() && id.len() <= 256,
                "invalid legacy operation id"
            );
            let inserted = transaction.execute(
                "INSERT OR IGNORE INTO outbox_import_receipts (source_id) VALUES (?1)",
                [id],
            )?;
            if inserted == 1 {
                super::coalesce::coalesce_cursor(&transaction, operation)?;
                let mut entry = halogen_sync_enrich::JournalEntry::new(operation.clone());
                entry.source_id = Some(id.clone());
                transaction.execute(
                    "INSERT INTO outbox (op) VALUES (?1)",
                    [serde_json::to_string(&entry)?],
                )?;
            }
        }
        transaction.commit()?;
        Ok(())
    }

    /// Clear the cache tables (keeping the durable outbox) when the on-disk
    /// cache shape predates [`CACHE_SCHEMA_VERSION`], then stamp the version.
    fn reset_stale_cache(&self) -> Result<()> {
        let mut conn = self.conn.borrow_mut();
        let version: i32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version == CACHE_SCHEMA_VERSION {
            return Ok(());
        }
        tracing::warn!(
            from = version,
            to = CACHE_SCHEMA_VERSION,
            "local store cache shape changed; clearing cached rows (outbox preserved)"
        );
        let tx = conn.transaction()?;
        for table in [
            "playbacks",
            "episodes",
            "podcasts",
            "playlists",
            "sync_metadata",
            "podcast_auto_playlists",
        ] {
            tx.execute(&format!("DELETE FROM {table}"), [])?;
        }
        tx.commit()?;
        conn.pragma_update(None, "user_version", CACHE_SCHEMA_VERSION)?;
        Ok(())
    }

    /// Clear the cached-content tables only (podcasts/episodes/playbacks/
    /// playlists), preserving the durable outbox — the native backend of the
    /// `/cache-control` "clear content" failsafe card (which bypasses the
    /// worker and opens the DB directly).
    pub fn clear_cached_content(&self) -> Result<()> {
        let mut conn = self.conn.borrow_mut();
        let tx = conn.transaction()?;
        for table in [
            "playbacks",
            "episodes",
            "podcasts",
            "playlists",
            "sync_metadata",
            "podcast_auto_playlists",
        ] {
            tx.execute(&format!("DELETE FROM {table}"), [])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Delete every pending outbox op, returning how many were dropped — the
    /// native backend of the `/cache-control` "clear sync queue" failsafe card.
    pub fn clear_outbox_rows(&self) -> Result<usize> {
        let n = self.conn.borrow_mut().execute("DELETE FROM outbox", [])?;
        Ok(n)
    }

    fn create_tables(&self) -> Result<()> {
        self.conn.borrow_mut().execute_batch(
            "
            CREATE TABLE IF NOT EXISTS podcasts (
                id   INTEGER PRIMARY KEY,
                data TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS episodes (
                id           INTEGER PRIMARY KEY,
                podcast_id   INTEGER NOT NULL,
                data         TEXT NOT NULL,
                published_at INTEGER
            );
            CREATE INDEX IF NOT EXISTS idx_episodes_podcast ON episodes(podcast_id);
            CREATE INDEX IF NOT EXISTS idx_episodes_published ON episodes(published_at);

            CREATE TABLE IF NOT EXISTS playbacks (
                episode_id INTEGER PRIMARY KEY,
                data       TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS outbox (
                id   INTEGER PRIMARY KEY AUTOINCREMENT,
                op   TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS playlists (
                id   INTEGER PRIMARY KEY,
                data TEXT NOT NULL
            );
            ",
        )?;
        Ok(())
    }
}

mod rows;
use rows::{from_json, load_json_rows};
mod records;
#[cfg(test)]
mod tests;
