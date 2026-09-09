use super::*;

impl CacheDatabase {
    /// Load + deserialize every cached episode (for filtered/non-`published_at`
    /// pages and filtered counts, which can't be expressed against the JSON blob
    /// in SQL).
    pub(super) fn load_all_episodes(&self) -> Result<Vec<EpisodeData>> {
        let conn = self.conn.borrow_mut();
        load_json_rows(&conn, "SELECT data FROM episodes", [])
    }

    /// Upsert a batch of `(id, json)`-keyed DTOs into a two-column `(id, data)`
    /// table in one transaction — the single place the per-collection upsert loop
    /// lives (podcasts/playlists). Episodes keep their own loop (extra
    /// `podcast_id`/`published_at` index columns).
    pub(super) fn upsert_rows<T: serde::Serialize>(
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
    pub(super) fn delete_by_ids(&self, table: &str, key_col: &str, ids: &[i32]) -> Result<()> {
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
pub(super) fn from_json<T: serde::de::DeserializeOwned>(s: String) -> rusqlite::Result<T> {
    serde_json::from_str(&s).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })
}

/// Run a `SELECT data …` query and deserialize each JSON-blob row into `T` — the
/// single place the prepare → `query_map(from_json)` → collect dance lives, shared
/// by every `list_*`/`load_*` read (each passing its own SQL + bound params).
pub(super) fn load_json_rows<T: serde::de::DeserializeOwned>(
    conn: &rusqlite::Connection,
    sql: &str,
    params: impl rusqlite::Params,
) -> Result<Vec<T>> {
    let mut stmt = conn.prepare(sql)?;
    let raw = stmt
        .query_map(params, |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<String>>>()?;
    // Per-row deserialize, SKIPPING undecodable rows: these are re-fetchable CACHE rows, and failing the whole
    // read over one corrupt/stale-shaped blob poisoned every list on this table until a manual wipe (the web
    // backend has always skipped bad rows). A skipped row is re-cached by the next fetch that includes it.
    let mut out = Vec::with_capacity(raw.len());
    let mut skipped = 0usize;
    for s in raw {
        match serde_json::from_str::<T>(&s) {
            Ok(v) => out.push(v),
            Err(_) => skipped += 1,
        }
    }
    if skipped > 0 {
        tracing::warn!(skipped, sql, "skipped undecodable cached rows");
    }
    Ok(out)
}
