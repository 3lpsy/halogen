use super::CacheDatabase;
use anyhow::Result;
use halogen_wire::PodcastAutoPlaylistData;
use rusqlite::OptionalExtension;
use std::collections::BTreeMap;

impl CacheDatabase {
    pub fn sync_cursor(&self) -> Result<Option<String>> {
        Ok(self
            .conn
            .borrow()
            .query_row(
                "SELECT value FROM sync_metadata WHERE key = 'cursor'",
                [],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub fn list_auto_playlists(&self) -> Result<BTreeMap<i32, Vec<PodcastAutoPlaylistData>>> {
        let connection = self.conn.borrow();
        let mut query = connection.prepare("SELECT podcast_id,data FROM podcast_auto_playlists")?;
        let rows = query.query_map([], |row| {
            Ok((row.get::<_, i32>(0)?, row.get::<_, String>(1)?))
        })?;
        rows.map(|row| {
            let (id, data) = row?;
            Ok((id, serde_json::from_str(&data)?))
        })
        .collect()
    }
}
