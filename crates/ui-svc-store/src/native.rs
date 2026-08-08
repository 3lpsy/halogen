//! Native `LocalStore` backed by a SQLite file.
//!
//! Records are stored as JSON blobs of the `halogen_wire` DTOs (keyed by id),
//! exactly like the web `localStorage` store. This keeps the store DTO-agnostic —
//! it never has to track the DTO field layout in SQL columns — so the schema can't
//! drift out of sync with the data types. Indexed columns are kept only where we
//! query by them (`episodes.podcast_id` and `episodes.published_at`).

use anyhow::Result;
use async_trait::async_trait;
use halogen_wire::{EpisodeData, PlaybackData, PlaylistData, PodcastData};
use rusqlite::Connection;
use std::path::PathBuf;

use super::outbox::OutboxOp;
use super::{
    EpisodeOrder, EpisodeQuery, EpisodeQueryFilter, LocalStore, filter_count, filter_sort_paginate,
};

/// Cache-shape version, stamped into SQLite's `user_version`. Bump when a
/// cached DTO's JSON shape changes incompatibly: on the next open the CACHE
/// tables (podcasts/episodes/playbacks/playlists — all re-fetchable) are
/// cleared and re-pulled, while the durable outbox is preserved. The native
/// mirror of the web store's `SCHEMA_VERSION` Cache-drop semantics — without
/// it, native had no reset path for stale-shaped rows at all.
const CACHE_SCHEMA_VERSION: i32 = 1;

/// Native LocalStore backed by a SQLite file (JSON-blob rows).
pub struct NativeLocalStore {
    conn: std::cell::RefCell<Connection>,
}

impl NativeLocalStore {
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
        store.reset_stale_cache()?;
        Ok(store)
    }

    /// Clear the cache tables (keeping the durable outbox) when the on-disk
    /// cache shape predates [`CACHE_SCHEMA_VERSION`], then stamp the version.
    fn reset_stale_cache(&self) -> Result<()> {
        let mut conn = self.conn.borrow_mut();
        let version: i32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version == CACHE_SCHEMA_VERSION {
            return Ok(());
        }
        halogen_ui_logging::warn!(
            from = version,
            to = CACHE_SCHEMA_VERSION,
            "local store cache shape changed; clearing cached rows (outbox preserved)"
        );
        let tx = conn.transaction()?;
        for table in ["playbacks", "episodes", "podcasts", "playlists"] {
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
        for table in ["playbacks", "episodes", "podcasts", "playlists"] {
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

impl NativeLocalStore {
    /// Load + deserialize every cached episode (for filtered/non-`published_at`
    /// pages and filtered counts, which can't be expressed against the JSON blob
    /// in SQL).
    fn load_all_episodes(&self) -> Result<Vec<EpisodeData>> {
        let conn = self.conn.borrow_mut();
        load_json_rows(&conn, "SELECT data FROM episodes", [])
    }

    /// Upsert a batch of `(id, json)`-keyed DTOs into a two-column `(id, data)`
    /// table in one transaction — the single place the per-collection upsert loop
    /// lives (podcasts/playlists). Episodes keep their own loop (extra
    /// `podcast_id`/`published_at` index columns).
    fn upsert_rows<T: serde::Serialize>(
        &self,
        table: &str,
        items: &[T],
        id_of: impl Fn(&T) -> i32,
    ) -> Result<()> {
        let mut conn = self.conn.borrow_mut();
        let tx = conn.transaction()?;
        let sql = format!("INSERT OR REPLACE INTO {table} (id, data) VALUES (?1, ?2)");
        for item in items {
            tx.execute(
                &sql,
                rusqlite::params![id_of(item), serde_json::to_string(item)?],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Delete every row of `table` whose `key_col` is in `ids`, in one transaction
    /// (no-op on an empty slice). The single place the prune primitives'
    /// `DELETE … WHERE <key> IN (…)` lives.
    fn delete_by_ids(&self, table: &str, key_col: &str, ids: &[i32]) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let placeholders = vec!["?"; ids.len()].join(",");
        let sql = format!("DELETE FROM {table} WHERE {key_col} IN ({placeholders})");
        self.conn
            .borrow_mut()
            .execute(&sql, rusqlite::params_from_iter(ids.iter()))?;
        Ok(())
    }
}

/// Parse a JSON-blob column into a DTO, mapping serde errors to a rusqlite error.
fn from_json<T: serde::de::DeserializeOwned>(s: String) -> rusqlite::Result<T> {
    serde_json::from_str(&s).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })
}

/// Run a `SELECT data …` query and deserialize each JSON-blob row into `T` — the
/// single place the prepare → `query_map(from_json)` → collect dance lives, shared
/// by every `list_*`/`load_*` read (each passing its own SQL + bound params).
fn load_json_rows<T: serde::de::DeserializeOwned>(
    conn: &rusqlite::Connection,
    sql: &str,
    params: impl rusqlite::Params,
) -> Result<Vec<T>> {
    let mut stmt = conn.prepare(sql)?;
    let raw = stmt
        .query_map(params, |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<String>>>()?;
    // Per-row deserialize, SKIPPING undecodable rows: these are re-fetchable
    // CACHE rows, and failing the whole read over one corrupt/stale-shaped blob
    // poisoned every list on this table until a manual wipe (the web backend
    // has always skipped bad rows). A skipped row is re-cached by the next
    // fetch that includes it.
    let mut out = Vec::with_capacity(raw.len());
    let mut skipped = 0usize;
    for s in raw {
        match serde_json::from_str::<T>(&s) {
            Ok(v) => out.push(v),
            Err(_) => skipped += 1,
        }
    }
    if skipped > 0 {
        halogen_ui_logging::warn!(skipped, sql, "skipped undecodable cached rows");
    }
    Ok(out)
}

#[async_trait(?Send)]
impl LocalStore for NativeLocalStore {
    async fn upsert_podcasts(&self, podcasts: &[PodcastData]) -> Result<()> {
        self.upsert_rows("podcasts", podcasts, |p| p.id)
    }

    async fn list_podcasts(&self) -> Result<Vec<PodcastData>> {
        let conn = self.conn.borrow_mut();
        load_json_rows(&conn, "SELECT data FROM podcasts", [])
    }

    async fn upsert_episodes(&self, episodes: &[EpisodeData]) -> Result<()> {
        let mut conn = self.conn.borrow_mut();
        let tx = conn.transaction()?;
        for e in episodes {
            tx.execute(
                "INSERT OR REPLACE INTO episodes (id, podcast_id, data, published_at) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![
                    e.id,
                    e.podcast_id,
                    serde_json::to_string(e)?,
                    e.published_at.map(|t| t.timestamp())
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    async fn list_episodes(&self, podcast_id: i32) -> Result<Vec<EpisodeData>> {
        let conn = self.conn.borrow_mut();
        load_json_rows(
            &conn,
            "SELECT data FROM episodes WHERE podcast_id = ?1",
            [podcast_id],
        )
    }

    async fn list_episodes_page(&self, query: &EpisodeQuery) -> Result<Vec<EpisodeData>> {
        // Fast path: the unfiltered `published_at` feed (`/latest` default) pages in
        // SQL with LIMIT/OFFSET on the indexed column. NULLs sort last under DESC
        // (episodes with no publish date fall to the end). Any filter, or another
        // order, can't be expressed against the JSON blob in SQL, so we load the
        // pool and filter/sort/slice in memory (matching the shared web path).
        if query.order_by == EpisodeOrder::PublishedAt && query.filter.is_empty() {
            let conn = self.conn.borrow_mut();
            let dir = if query.descending { "DESC" } else { "ASC" };
            let sql = format!(
                "SELECT data FROM episodes ORDER BY published_at {dir}, id {dir} LIMIT ?1 OFFSET ?2"
            );
            let limit = query.size.max(1) as i64;
            let offset = (query.page.max(0) as i64) * limit;
            return load_json_rows(&conn, &sql, [limit, offset]);
        }
        let all = self.load_all_episodes()?;
        Ok(filter_sort_paginate(all, query))
    }

    async fn count_episodes(&self, filter: &EpisodeQueryFilter) -> Result<usize> {
        // Unfiltered → cheap SQL COUNT; otherwise count the filtered pool in memory
        // (the predicate touches JSON-blob fields a column COUNT can't see).
        if filter.is_empty() {
            let conn = self.conn.borrow_mut();
            let n: i64 = conn.query_row("SELECT COUNT(*) FROM episodes", [], |r| r.get(0))?;
            return Ok(n.max(0) as usize);
        }
        Ok(filter_count(&self.load_all_episodes()?, filter))
    }

    async fn episodes_by_ids(&self, ids: &[i32]) -> Result<Vec<EpisodeData>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.conn.borrow_mut();
        // Build `id IN (?,?,…)` with one placeholder per id (all bound).
        let placeholders = vec!["?"; ids.len()].join(",");
        let sql = format!("SELECT data FROM episodes WHERE id IN ({placeholders})");
        load_json_rows(&conn, &sql, rusqlite::params_from_iter(ids.iter()))
    }

    async fn upsert_playlists(&self, playlists: &[PlaylistData]) -> Result<()> {
        self.upsert_rows("playlists", playlists, |p| p.id)
    }

    async fn list_playlists(&self) -> Result<Vec<PlaylistData>> {
        let conn = self.conn.borrow_mut();
        load_json_rows(&conn, "SELECT data FROM playlists", [])
    }

    async fn delete_playlist_row(&self, playlist_id: i32) -> Result<()> {
        self.conn
            .borrow_mut()
            .execute("DELETE FROM playlists WHERE id = ?1", [playlist_id])?;
        Ok(())
    }

    async fn save_playback(&self, playback: &PlaybackData) -> Result<()> {
        self.conn.borrow_mut().execute(
            "INSERT OR REPLACE INTO playbacks (episode_id, data) VALUES (?1, ?2)",
            rusqlite::params![playback.episode_id, serde_json::to_string(playback)?],
        )?;
        Ok(())
    }

    async fn list_playbacks(&self) -> Result<Vec<PlaybackData>> {
        let conn = self.conn.borrow_mut();
        load_json_rows(&conn, "SELECT data FROM playbacks", [])
    }

    // ── Prune primitives ────────────────────────────────────────────────
    // The cascade order/policy lives in the `LocalStore` trait defaults; these
    // just delete the rows it names. Each is its own (small) statement — the
    // cross-step cascade is no longer one transaction, which is fine here: these
    // are simple local-cache DELETEs that re-populate on the next pull, and the
    // web backend never had cross-step atomicity either.

    async fn list_episode_ids_for_podcast(&self, podcast_id: i32) -> Result<Vec<i32>> {
        let conn = self.conn.borrow_mut();
        let mut stmt = conn.prepare("SELECT id FROM episodes WHERE podcast_id = ?1")?;
        let ids = stmt
            .query_map([podcast_id], |row| row.get::<_, i32>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(ids)
    }

    async fn delete_episode_rows(&self, episode_ids: &[i32]) -> Result<()> {
        self.delete_by_ids("episodes", "id", episode_ids)
    }

    async fn delete_playback_rows(&self, episode_ids: &[i32]) -> Result<()> {
        self.delete_by_ids("playbacks", "episode_id", episode_ids)
    }

    async fn delete_podcast_row(&self, podcast_id: i32) -> Result<()> {
        self.conn
            .borrow_mut()
            .execute("DELETE FROM podcasts WHERE id = ?1", [podcast_id])?;
        Ok(())
    }

    async fn enqueue(&self, op: &OutboxOp) -> Result<()> {
        self.conn.borrow_mut().execute(
            "INSERT INTO outbox (op) VALUES (?1)",
            rusqlite::params![serde_json::to_string(op)?],
        )?;
        Ok(())
    }

    async fn pending(&self) -> Result<Vec<(u64, OutboxOp)>> {
        let conn = self.conn.borrow_mut();
        let mut stmt = conn.prepare("SELECT id, op FROM outbox ORDER BY id")?;
        let rows = stmt
            .query_map([], |row| {
                let id: i64 = row.get(0)?;
                Ok((id as u64, from_json::<OutboxOp>(row.get(1)?)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    async fn ack(&self, op_id: u64) -> Result<()> {
        self.conn
            .borrow_mut()
            .execute("DELETE FROM outbox WHERE id = ?1", [op_id as i64])?;
        Ok(())
    }

    async fn clear(&self) -> Result<()> {
        // One transaction so a mid-loop failure can't leave a half-wiped store
        // (e.g. episodes gone but the outbox kept) — all-or-nothing, like the
        // other multi-statement methods.
        let mut conn = self.conn.borrow_mut();
        let tx = conn.transaction()?;
        for table in ["playbacks", "episodes", "podcasts", "playlists", "outbox"] {
            tx.execute(&format!("DELETE FROM {table}"), [])?;
        }
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    fn temp_store() -> NativeLocalStore {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let path =
            std::env::temp_dir().join(format!("halogen_test_{}_{}.db", std::process::id(), n));
        let _ = std::fs::remove_file(&path);
        NativeLocalStore::open(path).unwrap()
    }

    /// Like [`temp_store`] but keeps the path so a test can reopen the same DB.
    fn temp_store_at() -> (NativeLocalStore, std::path::PathBuf) {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let path =
            std::env::temp_dir().join(format!("halogen_test_{}_r{}.db", std::process::id(), n));
        let _ = std::fs::remove_file(&path);
        (NativeLocalStore::open(path.clone()).unwrap(), path)
    }

    /// One corrupt cached row must be SKIPPED, not poison every read of the
    /// table (pre-fix a single bad blob failed all podcast/episode lists until
    /// a manual wipe; web has always skipped bad rows).
    #[tokio::test]
    async fn corrupt_cache_row_is_skipped_not_fatal() {
        let store = temp_store();
        store.upsert_podcasts(&[sample_podcast(1)]).await.unwrap();
        store
            .conn
            .borrow_mut()
            .execute(
                "INSERT INTO podcasts (id, data) VALUES (999, 'not json')",
                [],
            )
            .unwrap();
        let rows = store.list_podcasts().await.expect("read survives bad row");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, 1);
    }

    /// A cache-shape version bump clears the CACHE tables on reopen but keeps
    /// the durable outbox — the native mirror of the web `SCHEMA_VERSION`
    /// Cache-drop semantics.
    #[tokio::test]
    async fn stale_cache_version_wipes_cache_but_keeps_outbox() {
        let (store, path) = temp_store_at();
        store.upsert_podcasts(&[sample_podcast(1)]).await.unwrap();
        store
            .enqueue(&OutboxOp::MarkPlayed {
                episode_id: 7,
                played: true,
            })
            .await
            .unwrap();
        // Simulate a DB written by an older cache shape.
        store
            .conn
            .borrow_mut()
            .pragma_update(None, "user_version", CACHE_SCHEMA_VERSION - 1)
            .unwrap();
        drop(store);

        let store = NativeLocalStore::open(path).unwrap();
        assert!(
            store.list_podcasts().await.unwrap().is_empty(),
            "stale cache cleared"
        );
        assert_eq!(
            store.pending().await.unwrap().len(),
            1,
            "durable outbox preserved"
        );
    }

    fn sample_podcast(id: i32) -> PodcastData {
        let now = chrono::Utc::now();
        PodcastData {
            id,
            title: format!("Podcast {id}"),
            description: String::new(),
            feed_url: format!("https://example.com/{id}.xml"),
            art_url: None,
            art_file_path: None,
            author: None,
            etag: None,
            last_modified: None,
            polled_at: None,
            podcast_config_id: None,
            podcast_config: None,
            created_at: now,
            updated_at: now,
            episode_count: None,
            feed_url_redirects: None,
        }
    }

    fn sample_episode(id: i32, podcast_id: i32) -> EpisodeData {
        let now = chrono::Utc::now();
        EpisodeData {
            id,
            podcast_id,
            title: format!("Episode {id}"),
            description: Some("desc".into()),
            content_url: "https://example.com/a.mp3".into(),
            guid: None,
            art_url: None,
            published_at: Some(now),
            downloaded_at: None,
            content_file_path: None,
            download_size: None,
            art_file_path: None,
            download_status: halogen_wire::DownloadStatus::NotDownloaded,
            download_started_at: None,
            download_attempts: 0,
            playback_status: halogen_wire::PlaybackStatus::Unplayed,
            duration_secs: Some(123),
            created_at: now,
            updated_at: now,
            podcast: None,
            playback: None,
            chapters: None,
        }
    }

    #[tokio::test]
    async fn podcasts_episodes_playbacks_roundtrip() {
        let store = temp_store();

        store
            .upsert_podcasts(&[sample_podcast(1), sample_podcast(2)])
            .await
            .unwrap();
        let mut got = store.list_podcasts().await.unwrap();
        got.sort_by_key(|p| p.id);
        assert_eq!(got.len(), 2);
        assert_eq!(got[1].feed_url, "https://example.com/2.xml");

        // Upsert replaces (no duplicate) and round-trips duration_secs.
        store
            .upsert_episodes(&[
                sample_episode(10, 1),
                sample_episode(11, 1),
                sample_episode(20, 2),
            ])
            .await
            .unwrap();
        store
            .upsert_episodes(&[sample_episode(10, 1)])
            .await
            .unwrap();
        let eps_p1 = store.list_episodes(1).await.unwrap();
        assert_eq!(
            eps_p1.len(),
            2,
            "episodes filtered by podcast, deduped by id"
        );
        assert_eq!(eps_p1[0].duration_secs, Some(123));
        assert_eq!(store.list_episodes(2).await.unwrap().len(), 1);

        let pb = PlaybackData {
            id: 1,
            user_id: 1,
            episode_id: 10,
            cursor: 42,
            completed: false,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        store.save_playback(&pb).await.unwrap();
        let mut pb2 = pb.clone();
        pb2.cursor = 99;
        store.save_playback(&pb2).await.unwrap();
        let pbs = store.list_playbacks().await.unwrap();
        assert_eq!(pbs.len(), 1, "playback upserts on episode_id");
        assert_eq!(pbs[0].cursor, 99);
    }

    #[tokio::test]
    async fn list_episodes_page_orders_and_offsets() {
        use chrono::{TimeZone, Utc};

        let store = temp_store();
        // Three podcasts' episodes interleaved; distinct publish years so the
        // expected newest-first order spans podcasts (cross-podcast paging).
        let mut eps = Vec::new();
        for (id, year) in [(10, 2020), (11, 2023), (20, 2021), (21, 2022)] {
            let mut e = sample_episode(id, if id < 20 { 1 } else { 2 });
            e.published_at = Some(Utc.with_ymd_and_hms(year, 1, 1, 0, 0, 0).unwrap());
            eps.push(e);
        }
        store.upsert_episodes(&eps).await.unwrap();

        let q = |page| EpisodeQuery {
            order_by: EpisodeOrder::PublishedAt,
            descending: true,
            page,
            size: 2,
            filter: EpisodeQueryFilter::default(),
        };
        // Page 0: two newest (2023, 2022); page 1: next two (2021, 2020).
        let p0 = store.list_episodes_page(&q(0)).await.unwrap();
        assert_eq!(p0.iter().map(|e| e.id).collect::<Vec<_>>(), vec![11, 21]);
        let p1 = store.list_episodes_page(&q(1)).await.unwrap();
        assert_eq!(p1.iter().map(|e| e.id).collect::<Vec<_>>(), vec![20, 10]);
        // Past the end → empty, signalling no more pages.
        assert!(store.list_episodes_page(&q(2)).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn list_episodes_page_title_order_uses_in_memory_fallback() {
        let store = temp_store();
        // Titles out of id order so a pure id sort would fail; only the Title
        // fallback path (non-indexed column) yields alphabetical.
        let mut eps = Vec::new();
        for (id, title) in [(10, "Charlie"), (11, "Alpha"), (12, "Bravo")] {
            let mut e = sample_episode(id, 1);
            e.title = title.to_string();
            eps.push(e);
        }
        store.upsert_episodes(&eps).await.unwrap();

        let page = store
            .list_episodes_page(&EpisodeQuery {
                order_by: EpisodeOrder::Title,
                descending: false,
                page: 0,
                size: 10,
                filter: EpisodeQueryFilter::default(),
            })
            .await
            .unwrap();
        assert_eq!(
            page.iter().map(|e| e.title.as_str()).collect::<Vec<_>>(),
            vec!["Alpha", "Bravo", "Charlie"],
        );
    }

    #[tokio::test]
    async fn playlists_and_episodes_by_ids_roundtrip() {
        let store = temp_store();
        store
            .upsert_episodes(&[
                sample_episode(10, 1),
                sample_episode(11, 1),
                sample_episode(12, 1),
            ])
            .await
            .unwrap();

        let now = chrono::Utc::now();
        let pl = PlaylistData {
            id: 5,
            name: "PL".into(),
            description: None,
            is_default: false,
            position: 0,
            on_remove_delete_file_server: false,
            on_remove_delete_file_client: false,
            created_at: now,
            updated_at: now,
            episode_ids: Some(vec![12, 10]),
            episode_playlist: None,
        };
        store.upsert_playlists(&[pl]).await.unwrap();

        let got = store.list_playlists().await.unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(
            got[0].episode_ids,
            Some(vec![12, 10]),
            "ordered ids persist"
        );

        // Resolve bodies by id, ignoring misses (999 absent).
        let mut ids: Vec<i32> = store
            .episodes_by_ids(&[12, 10, 999])
            .await
            .unwrap()
            .iter()
            .map(|e| e.id)
            .collect();
        ids.sort();
        assert_eq!(ids, vec![10, 12]);
    }

    #[tokio::test]
    async fn outbox_fifo_enqueue_pending_ack() {
        let store = temp_store();
        store
            .enqueue(&OutboxOp::MarkPlayed {
                episode_id: 1,
                played: true,
            })
            .await
            .unwrap();
        store
            .enqueue(&OutboxOp::SetCursor {
                episode_id: 2,
                cursor: 5,
            })
            .await
            .unwrap();

        let pending = store.pending().await.unwrap();
        assert_eq!(pending.len(), 2);
        assert!(pending[0].0 < pending[1].0, "outbox should be FIFO by id");

        let first = pending[0].0;
        store.ack(first).await.unwrap();
        let pending = store.pending().await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_ne!(pending[0].0, first);
    }

    /// Playback row for `episode_id` (cursor unused by the prune tests).
    fn sample_playback(id: i32, episode_id: i32) -> PlaybackData {
        let now = chrono::Utc::now();
        PlaybackData {
            id,
            user_id: 1,
            episode_id,
            cursor: 0,
            completed: false,
            created_at: now,
            updated_at: now,
        }
    }

    #[tokio::test]
    async fn delete_episode_removes_row_and_playback() {
        let store = temp_store();
        store
            .upsert_episodes(&[sample_episode(10, 1), sample_episode(11, 1)])
            .await
            .unwrap();
        store.save_playback(&sample_playback(1, 10)).await.unwrap();
        store.save_playback(&sample_playback(2, 11)).await.unwrap();

        store.delete_episode(10).await.unwrap();

        // The targeted episode + its playback are gone; the sibling survives.
        let eps: Vec<i32> = store
            .list_episodes(1)
            .await
            .unwrap()
            .iter()
            .map(|e| e.id)
            .collect();
        assert_eq!(eps, vec![11]);
        let pb_eps: Vec<i32> = store
            .list_playbacks()
            .await
            .unwrap()
            .iter()
            .map(|p| p.episode_id)
            .collect();
        assert_eq!(pb_eps, vec![11]);
    }

    #[tokio::test]
    async fn delete_podcast_removes_podcast_episodes_and_playbacks() {
        let store = temp_store();
        store
            .upsert_podcasts(&[sample_podcast(1), sample_podcast(2)])
            .await
            .unwrap();
        store
            .upsert_episodes(&[
                sample_episode(10, 1),
                sample_episode(11, 1),
                sample_episode(20, 2),
            ])
            .await
            .unwrap();
        store.save_playback(&sample_playback(1, 10)).await.unwrap();
        store.save_playback(&sample_playback(2, 20)).await.unwrap();

        store.delete_podcast(1).await.unwrap();

        // Podcast 1 and all its episodes/playbacks gone; podcast 2 untouched.
        let pods: Vec<i32> = store
            .list_podcasts()
            .await
            .unwrap()
            .iter()
            .map(|p| p.id)
            .collect();
        assert_eq!(pods, vec![2]);
        assert!(store.list_episodes(1).await.unwrap().is_empty());
        assert_eq!(store.list_episodes(2).await.unwrap().len(), 1);
        let pb_eps: Vec<i32> = store
            .list_playbacks()
            .await
            .unwrap()
            .iter()
            .map(|p| p.episode_id)
            .collect();
        assert_eq!(
            pb_eps,
            vec![20],
            "only the surviving podcast's playback remains"
        );
    }

    #[tokio::test]
    async fn clear_empties_store() {
        let store = temp_store();
        store.upsert_podcasts(&[sample_podcast(1)]).await.unwrap();
        store
            .upsert_episodes(&[sample_episode(10, 1)])
            .await
            .unwrap();
        store
            .enqueue(&OutboxOp::Subscribe {
                feed_url: "https://example.com/feed".into(),
                title: None,
                description: None,
                author: None,
            })
            .await
            .unwrap();
        store.clear().await.unwrap();
        assert!(store.pending().await.unwrap().is_empty());
        assert!(store.list_podcasts().await.unwrap().is_empty());
        assert!(store.list_episodes(1).await.unwrap().is_empty());
    }
}
