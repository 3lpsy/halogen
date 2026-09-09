use crate::{EpisodeQuery, EpisodeQueryFilter, LocalStore, OutboxOp};
use anyhow::Result;
use async_trait::async_trait;
use halogen_queries::cache::CacheDatabase;
use halogen_wire::{EpisodeData, PlaybackData, PlaylistData, PodcastData};
use std::path::PathBuf;

pub struct NativeLocalStore {
    backend: CacheDatabase,
}
impl NativeLocalStore {
    pub fn open(path: PathBuf) -> Result<Self> {
        Ok(Self {
            backend: CacheDatabase::open(path)?,
        })
    }
    pub fn import_operations(&self, operations: &[(String, OutboxOp)]) -> Result<()> {
        self.backend.import_operations(operations)
    }
    pub fn clear_cached_content(&self) -> Result<()> {
        self.backend.clear_cached_content()
    }
    pub fn clear_outbox_rows(&self) -> Result<usize> {
        self.backend.clear_outbox_rows()
    }
}
#[async_trait(?Send)]
impl LocalStore for NativeLocalStore {
    async fn sync_cursor(&self) -> Result<Option<String>> {
        self.backend.sync_cursor()
    }
    async fn list_auto_playlists(
        &self,
    ) -> Result<std::collections::BTreeMap<i32, Vec<halogen_wire::PodcastAutoPlaylistData>>> {
        self.backend.list_auto_playlists()
    }

    async fn commit_changes(
        &self,
        changes: &crate::StoreChanges,
        operations: &[OutboxOp],
    ) -> Result<()> {
        self.backend.commit_changes(changes, operations)
    }
    async fn upsert_podcasts(&self, podcasts: &[PodcastData]) -> Result<()> {
        self.backend.upsert_podcasts(podcasts)
    }
    async fn list_podcasts(&self) -> Result<Vec<PodcastData>> {
        self.backend.list_podcasts()
    }
    async fn upsert_episodes(&self, episodes: &[EpisodeData]) -> Result<()> {
        self.backend.upsert_episodes(episodes)
    }
    async fn list_episodes(&self, podcast_id: i32) -> Result<Vec<EpisodeData>> {
        self.backend.list_episodes(podcast_id)
    }
    async fn list_episodes_page(&self, query: &EpisodeQuery) -> Result<Vec<EpisodeData>> {
        self.backend.list_episodes_page(query)
    }
    async fn count_episodes(&self, filter: &EpisodeQueryFilter) -> Result<usize> {
        self.backend.count_episodes(filter)
    }
    async fn episodes_by_ids(&self, ids: &[i32]) -> Result<Vec<EpisodeData>> {
        self.backend.episodes_by_ids(ids)
    }
    async fn upsert_playlists(&self, playlists: &[PlaylistData]) -> Result<()> {
        self.backend.upsert_playlists(playlists)
    }
    async fn list_playlists(&self) -> Result<Vec<PlaylistData>> {
        self.backend.list_playlists()
    }
    async fn delete_playlist_row(&self, playlist_id: i32) -> Result<()> {
        self.backend.delete_playlist_row(playlist_id)
    }
    async fn save_playback(&self, playback: &PlaybackData) -> Result<()> {
        self.backend.save_playback(playback)
    }
    async fn list_playbacks(&self) -> Result<Vec<PlaybackData>> {
        self.backend.list_playbacks()
    }
    async fn list_episode_ids_for_podcast(&self, podcast_id: i32) -> Result<Vec<i32>> {
        self.backend.list_episode_ids_for_podcast(podcast_id)
    }
    async fn delete_episode_rows(&self, episode_ids: &[i32]) -> Result<()> {
        self.backend.delete_episode_rows(episode_ids)
    }
    async fn delete_playback_rows(&self, episode_ids: &[i32]) -> Result<()> {
        self.backend.delete_playback_rows(episode_ids)
    }
    async fn delete_podcast_row(&self, podcast_id: i32) -> Result<()> {
        self.backend.delete_podcast_row(podcast_id)
    }
    async fn enqueue(&self, op: &OutboxOp) -> Result<()> {
        self.backend.enqueue(op)
    }
    async fn pending(&self) -> Result<Vec<(u64, OutboxOp)>> {
        self.backend.pending()
    }
    async fn journal_entries(&self) -> Result<Vec<(u64, crate::JournalEntry)>> {
        self.backend.journal_entries()
    }
    async fn save_journal_entry(&self, id: u64, entry: &crate::JournalEntry) -> Result<()> {
        self.backend.save_journal_entry(id, entry)
    }
    async fn ack(&self, op_id: u64) -> Result<()> {
        self.backend.ack(op_id)
    }
    async fn clear(&self) -> Result<()> {
        self.backend.clear()
    }
}
