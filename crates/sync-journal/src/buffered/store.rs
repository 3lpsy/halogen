use super::buffer::{BufferedStore, overlay};
use anyhow::{Result, bail};
use async_trait::async_trait;
use halogen_sync_store::{
    EpisodeQuery, EpisodeQueryFilter, JournalEntry, LocalStore, OutboxOp, StoreChanges,
    filter_count, filter_sort_paginate,
};
use halogen_wire::{EpisodeData, PlaybackData, PlaylistData, PodcastData};

#[async_trait(?Send)]
impl LocalStore for BufferedStore {
    async fn sync_cursor(&self) -> Result<Option<String>> {
        self.read(self.inner.sync_cursor().await)
    }
    async fn list_auto_playlists(
        &self,
    ) -> Result<std::collections::BTreeMap<i32, Vec<halogen_wire::PodcastAutoPlaylistData>>> {
        let mut rows = self.read(self.inner.list_auto_playlists().await)?;
        rows.extend(self.changes.borrow().auto_playlists.clone());
        Ok(rows)
    }
    async fn replace_auto_playlists(
        &self,
        podcast_id: i32,
        rows: &[halogen_wire::PodcastAutoPlaylistData],
    ) -> Result<()> {
        self.changes
            .borrow_mut()
            .auto_playlists
            .insert(podcast_id, rows.to_vec());
        Ok(())
    }
    async fn upsert_podcasts(&self, rows: &[PodcastData]) -> Result<()> {
        let mut changes = self.changes.borrow_mut();
        for row in rows {
            changes.deleted_podcasts.retain(|id| *id != row.id);
        }
        changes.podcasts.extend_from_slice(rows);
        Ok(())
    }
    async fn list_podcasts(&self) -> Result<Vec<PodcastData>> {
        let rows = self.read(self.inner.list_podcasts().await)?;
        let changes = self.changes.borrow();
        Ok(overlay(
            rows,
            &changes.podcasts,
            &changes.deleted_podcasts,
            |row| row.id,
        ))
    }
    async fn upsert_episodes(&self, rows: &[EpisodeData]) -> Result<()> {
        let mut changes = self.changes.borrow_mut();
        for row in rows {
            changes.deleted_episodes.retain(|id| *id != row.id);
        }
        changes.episodes.extend_from_slice(rows);
        Ok(())
    }
    async fn list_episodes(&self, podcast_id: i32) -> Result<Vec<EpisodeData>> {
        // Query only the affected podcast; command-time reads must not load the whole library.
        let rows = self.read(self.inner.list_episodes(podcast_id).await)?;
        let changes = self.changes.borrow();
        Ok(
            overlay(rows, &changes.episodes, &changes.deleted_episodes, |row| {
                row.id
            })
            .into_iter()
            .filter(|row| row.podcast_id == podcast_id)
            .collect(),
        )
    }
    async fn list_episodes_page(&self, query: &EpisodeQuery) -> Result<Vec<EpisodeData>> {
        Ok(filter_sort_paginate(self.episodes().await?, query))
    }
    async fn count_episodes(&self, filter: &EpisodeQueryFilter) -> Result<usize> {
        Ok(filter_count(&self.episodes().await?, filter))
    }
    async fn episodes_by_ids(&self, ids: &[i32]) -> Result<Vec<EpisodeData>> {
        let rows = self.read(self.inner.episodes_by_ids(ids).await)?;
        let changes = self.changes.borrow();
        Ok(
            overlay(rows, &changes.episodes, &changes.deleted_episodes, |row| {
                row.id
            })
            .into_iter()
            .filter(|row| ids.contains(&row.id))
            .collect(),
        )
    }
    async fn upsert_playlists(&self, rows: &[PlaylistData]) -> Result<()> {
        let mut changes = self.changes.borrow_mut();
        for row in rows {
            changes.deleted_playlists.retain(|id| *id != row.id);
        }
        changes.playlists.extend_from_slice(rows);
        Ok(())
    }
    async fn list_playlists(&self) -> Result<Vec<PlaylistData>> {
        let rows = self.read(self.inner.list_playlists().await)?;
        let changes = self.changes.borrow();
        Ok(overlay(
            rows,
            &changes.playlists,
            &changes.deleted_playlists,
            |row| row.id,
        ))
    }
    async fn delete_playlist_row(&self, id: i32) -> Result<()> {
        let mut changes = self.changes.borrow_mut();
        changes.playlists.retain(|row| row.id != id);
        changes.deleted_playlists.push(id);
        Ok(())
    }
    async fn save_playback(&self, row: &PlaybackData) -> Result<()> {
        let mut changes = self.changes.borrow_mut();
        changes.deleted_playbacks.retain(|id| *id != row.episode_id);
        changes.playbacks.push(row.clone());
        Ok(())
    }
    async fn list_playbacks(&self) -> Result<Vec<PlaybackData>> {
        let rows = self.read(self.inner.list_playbacks().await)?;
        let changes = self.changes.borrow();
        Ok(overlay(
            rows,
            &changes.playbacks,
            &changes.deleted_playbacks,
            |row| row.episode_id,
        ))
    }
    async fn list_episode_ids_for_podcast(&self, id: i32) -> Result<Vec<i32>> {
        Ok(self
            .list_episodes(id)
            .await?
            .into_iter()
            .map(|row| row.id)
            .collect())
    }
    async fn delete_episode_rows(&self, ids: &[i32]) -> Result<()> {
        let mut changes = self.changes.borrow_mut();
        changes.episodes.retain(|row| !ids.contains(&row.id));
        changes.deleted_episodes.extend_from_slice(ids);
        Ok(())
    }
    async fn delete_playback_rows(&self, ids: &[i32]) -> Result<()> {
        let mut changes = self.changes.borrow_mut();
        changes
            .playbacks
            .retain(|row| !ids.contains(&row.episode_id));
        changes.deleted_playbacks.extend_from_slice(ids);
        Ok(())
    }
    async fn delete_podcast_row(&self, id: i32) -> Result<()> {
        let mut changes = self.changes.borrow_mut();
        changes.podcasts.retain(|row| row.id != id);
        changes.deleted_podcasts.push(id);
        Ok(())
    }
    async fn enqueue(&self, op: &OutboxOp) -> Result<()> {
        self.operations.borrow_mut().push(Some(op.clone()));
        Ok(())
    }
    async fn pending(&self) -> Result<Vec<(u64, OutboxOp)>> {
        let mut rows = self.read(self.inner.pending().await)?;
        let changes = self.changes.borrow();
        rows.retain(|(id, _)| !changes.acknowledged_operations.contains(id));
        rows.extend(
            self.operations
                .borrow()
                .iter()
                .enumerate()
                .filter_map(|(index, op)| op.clone().map(|op| (u64::MAX - index as u64, op))),
        );
        Ok(rows)
    }
    async fn journal_entries(&self) -> Result<Vec<(u64, JournalEntry)>> {
        let mut rows = self.read(self.inner.journal_entries().await)?;
        let changes = self.changes.borrow();
        rows.retain(|(id, _)| !changes.acknowledged_operations.contains(id));
        rows.extend(
            self.operations
                .borrow()
                .iter()
                .enumerate()
                .filter_map(|(index, op)| {
                    op.clone()
                        .map(|op| (u64::MAX - index as u64, JournalEntry::new(op)))
                }),
        );
        Ok(rows)
    }
    async fn save_journal_entry(&self, _: u64, _: &JournalEntry) -> Result<()> {
        bail!("delivery metadata cannot change inside a command transaction")
    }
    async fn ack(&self, id: u64) -> Result<()> {
        let index = u64::MAX - id;
        let mut operations = self.operations.borrow_mut();
        if index < operations.len() as u64 {
            operations[index as usize] = None;
        } else {
            self.changes.borrow_mut().acknowledged_operations.push(id);
        }
        Ok(())
    }
    async fn commit_changes(&self, _: &StoreChanges, _: &[OutboxOp]) -> Result<()> {
        bail!("nested command transactions are unsupported")
    }
    async fn clear(&self) -> Result<()> {
        bail!("cache reset cannot run inside a command transaction")
    }
}
