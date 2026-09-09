use super::*;

impl CacheDatabase {
    pub fn commit_changes(
        &self,
        changes: &halogen_sync_enrich::StoreChanges,
        operations: &[OutboxOp],
    ) -> Result<()> {
        self.commit_transaction(changes, operations)
    }

    pub fn upsert_podcasts(&self, podcasts: &[PodcastData]) -> Result<()> {
        self.upsert_rows("podcasts", podcasts, |p| p.id)
    }

    pub fn list_podcasts(&self) -> Result<Vec<PodcastData>> {
        let conn = self.conn.borrow_mut();
        load_json_rows(&conn, "SELECT data FROM podcasts", [])
    }

    pub fn upsert_episodes(&self, episodes: &[EpisodeData]) -> Result<()> {
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

    pub fn list_episodes(&self, podcast_id: i32) -> Result<Vec<EpisodeData>> {
        let conn = self.conn.borrow_mut();
        load_json_rows(
            &conn,
            "SELECT data FROM episodes WHERE podcast_id = ?1",
            [podcast_id],
        )
    }

    pub fn list_episodes_page(&self, query: &EpisodeQuery) -> Result<Vec<EpisodeData>> {
        // Fast path: the unfiltered `published_at` feed (`/latest` default) pages in SQL with LIMIT/OFFSET on
        // the indexed column. NULLs sort last under DESC (episodes with no publish date fall to the end). Any
        // filter, or another order, can't be expressed against the JSON blob in SQL, so we load the pool and
        // filter/sort/slice in memory (matching the shared web path).
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

    pub fn count_episodes(&self, filter: &EpisodeQueryFilter) -> Result<usize> {
        // Unfiltered → cheap SQL COUNT; otherwise count the filtered pool in memory
        // (the predicate touches JSON-blob fields a column COUNT can't see).
        if filter.is_empty() {
            let conn = self.conn.borrow_mut();
            let n: i64 = conn.query_row("SELECT COUNT(*) FROM episodes", [], |r| r.get(0))?;
            return Ok(n.max(0) as usize);
        }
        Ok(filter_count(&self.load_all_episodes()?, filter))
    }

    pub fn episodes_by_ids(&self, ids: &[i32]) -> Result<Vec<EpisodeData>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.conn.borrow_mut();
        // Build `id IN (?,?,…)` with one placeholder per id (all bound).
        let placeholders = vec!["?"; ids.len()].join(",");
        let sql = format!("SELECT data FROM episodes WHERE id IN ({placeholders})");
        load_json_rows(&conn, &sql, rusqlite::params_from_iter(ids.iter()))
    }

    pub fn upsert_playlists(&self, playlists: &[PlaylistData]) -> Result<()> {
        self.upsert_rows("playlists", playlists, |p| p.id)
    }

    pub fn list_playlists(&self) -> Result<Vec<PlaylistData>> {
        let conn = self.conn.borrow_mut();
        load_json_rows(&conn, "SELECT data FROM playlists", [])
    }

    pub fn delete_playlist_row(&self, playlist_id: i32) -> Result<()> {
        self.conn
            .borrow_mut()
            .execute("DELETE FROM playlists WHERE id = ?1", [playlist_id])?;
        Ok(())
    }

    pub fn save_playback(&self, playback: &PlaybackData) -> Result<()> {
        self.conn.borrow_mut().execute(
            "INSERT OR REPLACE INTO playbacks (episode_id, data) VALUES (?1, ?2)",
            rusqlite::params![playback.episode_id, serde_json::to_string(playback)?],
        )?;
        Ok(())
    }

    pub fn list_playbacks(&self) -> Result<Vec<PlaybackData>> {
        let conn = self.conn.borrow_mut();
        load_json_rows(&conn, "SELECT data FROM playbacks", [])
    }

    // ── Prune primitives ──────────────────────────────────────────────── The cascade order/policy lives in
    // the `LocalStore` trait defaults; these just delete the rows it names. Each is its own (small) statement —
    // the cross-step cascade is no longer one transaction, which is fine here: these are simple local-cache
    // DELETEs that re-populate on the next pull, and the web backend never had cross-step atomicity either.

    pub fn list_episode_ids_for_podcast(&self, podcast_id: i32) -> Result<Vec<i32>> {
        let conn = self.conn.borrow_mut();
        let mut stmt = conn.prepare("SELECT id FROM episodes WHERE podcast_id = ?1")?;
        let ids = stmt
            .query_map([podcast_id], |row| row.get::<_, i32>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(ids)
    }

    pub fn delete_episode_rows(&self, episode_ids: &[i32]) -> Result<()> {
        self.delete_by_ids("episodes", "id", episode_ids)
    }

    pub fn delete_playback_rows(&self, episode_ids: &[i32]) -> Result<()> {
        self.delete_by_ids("playbacks", "episode_id", episode_ids)
    }

    pub fn delete_podcast_row(&self, podcast_id: i32) -> Result<()> {
        self.conn
            .borrow_mut()
            .execute("DELETE FROM podcasts WHERE id = ?1", [podcast_id])?;
        Ok(())
    }

    pub fn enqueue(&self, op: &OutboxOp) -> Result<()> {
        self.conn.borrow_mut().execute(
            "INSERT INTO outbox (op) VALUES (?1)",
            rusqlite::params![serde_json::to_string(op)?],
        )?;
        Ok(())
    }

    pub fn pending(&self) -> Result<Vec<(u64, OutboxOp)>> {
        Ok(self
            .journal_entries()?
            .into_iter()
            .filter(|(_, entry)| entry.rejection.is_none() && !entry.delivered)
            .map(|(id, entry)| (id, entry.operation))
            .collect())
    }

    pub fn journal_entries(&self) -> Result<Vec<(u64, halogen_sync_enrich::JournalEntry)>> {
        let conn = self.conn.borrow_mut();
        let mut stmt = conn.prepare("SELECT id, op FROM outbox ORDER BY id")?;
        let rows = stmt
            .query_map([], |row| {
                let id: i64 = row.get(0)?;
                Ok((
                    id as u64,
                    halogen_sync_enrich::JournalEntry::from(from_json::<
                        halogen_sync_enrich::StoredEntry,
                    >(row.get(1)?)?),
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn save_journal_entry(
        &self,
        id: u64,
        entry: &halogen_sync_enrich::JournalEntry,
    ) -> Result<()> {
        let id = i64::try_from(id)?;
        anyhow::ensure!(id > 0, "invalid journal id");
        let changed = self.conn.borrow_mut().execute(
            "UPDATE outbox SET op = ?1 WHERE id = ?2",
            rusqlite::params![serde_json::to_string(entry)?, id],
        )?;
        anyhow::ensure!(changed == 1, "journal entry no longer exists");
        Ok(())
    }

    pub fn ack(&self, op_id: u64) -> Result<()> {
        self.conn
            .borrow_mut()
            .execute("DELETE FROM outbox WHERE id = ?1", [op_id as i64])?;
        Ok(())
    }

    pub fn clear(&self) -> Result<()> {
        // One transaction so a mid-loop failure can't leave a half-wiped store
        // (e.g. episodes gone but the outbox kept) — all-or-nothing, like the
        // other multi-statement methods.
        let mut conn = self.conn.borrow_mut();
        let tx = conn.transaction()?;
        for table in [
            "playbacks",
            "episodes",
            "podcasts",
            "playlists",
            "outbox",
            "sync_metadata",
            "podcast_auto_playlists",
        ] {
            tx.execute(&format!("DELETE FROM {table}"), [])?;
        }
        tx.commit()?;
        Ok(())
    }
}

impl CacheDatabase {
    pub fn delete_episode(&self, id: i32) -> Result<()> {
        self.delete_playback_rows(&[id])?;
        self.delete_episode_rows(&[id])
    }
    pub fn delete_podcast(&self, id: i32) -> Result<()> {
        let episodes = self.list_episode_ids_for_podcast(id)?;
        self.delete_playback_rows(&episodes)?;
        self.delete_episode_rows(&episodes)?;
        self.delete_podcast_row(id)
    }
}
