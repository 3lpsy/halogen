//! Shared metadata persistence and atomic journal/cursor contract.

use std::rc::Rc;

use anyhow::Result;
use async_trait::async_trait;

use crate::{EpisodeQuery, EpisodeQueryFilter, OutboxOp};
use halogen_wire::{EpisodeData, PlaybackData, PlaylistData, PodcastData};

/// A cloneable handle to the one shared `LocalStore`, provided via context so the
/// worker (sole writer) and components (readers) observe the same backing store.
/// `None` when the store failed to open — sync/caching is then disabled.
#[derive(Clone)]
pub struct StoreHandle(pub Option<Rc<dyn LocalStore>>);

/// Platform persistence uses native rusqlite or per-account IndexedDB metadata records. Audio bytes live in a separate
/// media database.
#[async_trait(?Send)]
pub trait LocalStore {
    async fn sync_cursor(&self) -> Result<Option<String>>;
    async fn list_auto_playlists(
        &self,
    ) -> Result<std::collections::BTreeMap<i32, Vec<halogen_wire::PodcastAutoPlaylistData>>>;

    async fn replace_auto_playlists(
        &self,
        podcast_id: i32,
        rows: &[halogen_wire::PodcastAutoPlaylistData],
    ) -> Result<()> {
        let mut changes = crate::StoreChanges::default();
        changes.auto_playlists.insert(podcast_id, rows.to_vec());
        self.commit_changes(&changes, &[]).await
    }

    /// Apply cache changes and append their operations in a single transaction.
    async fn commit_changes(
        &self,
        changes: &crate::StoreChanges,
        operations: &[OutboxOp],
    ) -> Result<()>;

    // ── Podcasts ────────────────────────────────────────────────────────

    /// Upsert a batch of podcasts (INSERT … ON CONFLICT … UPDATE).
    async fn upsert_podcasts(&self, podcasts: &[PodcastData]) -> Result<()>;

    /// List all podcasts. Hydrates the in-memory pool (`podcasts_by_id`) on boot;
    /// the paged podcasts list renders a window over that pool (offline-first) and
    /// revalidates page-by-page from the server.
    async fn list_podcasts(&self) -> Result<Vec<PodcastData>>;

    // ── Episodes ────────────────────────────────────────────────────────

    /// Upsert a batch of episodes.
    async fn upsert_episodes(&self, episodes: &[EpisodeData]) -> Result<()>;

    /// List episodes for a specific podcast.
    async fn list_episodes(&self, podcast_id: i32) -> Result<Vec<EpisodeData>>;

    /// List one filtered + ordered page of episodes across all podcasts. The
    /// render source for server-paged views (e.g. `/latest`): the UI reads the
    /// cached page, then revalidates it from the server and upserts the result
    /// back here.
    async fn list_episodes_page(&self, query: &EpisodeQuery) -> Result<Vec<EpisodeData>>;

    /// Count cached episodes matching the query's `filter` (order/page ignored),
    /// so a paged view knows whether more pool rows remain to render before it
    /// fetches the next server page.
    async fn count_episodes(&self, filter: &EpisodeQueryFilter) -> Result<usize>;

    /// Resolve episode bodies for a set of ids from the pool (any order — the
    /// caller reorders, e.g. by playlist position). Missing ids are absent.
    async fn episodes_by_ids(&self, ids: &[i32]) -> Result<Vec<EpisodeData>>;

    // ── Playlists ───────────────────────────────────────────────────────

    /// Upsert a batch of playlists (each carrying its ordered `episode_ids`).
    async fn upsert_playlists(&self, playlists: &[PlaylistData]) -> Result<()>;

    /// List all cached playlists (with their `episode_ids`).
    async fn list_playlists(&self) -> Result<Vec<PlaylistData>>;

    /// Delete the cached playlist row for `playlist_id`. Its episode membership
    /// rides the row itself (`episode_ids`) — there is no separate cached episode
    /// list to prune. Mirrors a confirmed SERVER-side playlist delete into the
    /// local cache (unlike the local-only prune primitives below).
    async fn delete_playlist_row(&self, playlist_id: i32) -> Result<()>;

    // ── Playbacks ───────────────────────────────────────────────────────

    /// Save or update a playback position.
    async fn save_playback(&self, playback: &PlaybackData) -> Result<()>;

    /// List all playbacks.
    async fn list_playbacks(&self) -> Result<Vec<PlaybackData>>;

    // Local recovery prunes cached metadata/playback only; the worker handles audio blobs and in-memory membership.
    // Rows return on sync, with no server mutation. Default delete_episode/delete_podcast methods define shared prune
    // order using backend primitives so native and web policy cannot diverge.

    // ── Prune primitives (per-backend storage ops the shared policy is built on) ─

    /// Episode ids cached for `podcast_id`, in any order — the shared
    /// `delete_podcast` policy uses these to find the playbacks to prune (playbacks
    /// are keyed by `episode_id` with no podcast field, so they can't be resolved
    /// from the podcast id alone).
    async fn list_episode_ids_for_podcast(&self, podcast_id: i32) -> Result<Vec<i32>>;

    /// Delete the cached episode rows for `episode_ids` (no-op on an empty slice).
    /// Does NOT touch playbacks — the policy prunes those explicitly.
    async fn delete_episode_rows(&self, episode_ids: &[i32]) -> Result<()>;

    /// Delete the playback rows keyed by `episode_ids` (no-op on an empty slice).
    async fn delete_playback_rows(&self, episode_ids: &[i32]) -> Result<()>;

    /// Delete the cached podcast row for `podcast_id` (its episodes/playbacks are
    /// pruned separately by the policy).
    async fn delete_podcast_row(&self, podcast_id: i32) -> Result<()>;

    /// Delete one episode's cached row and its playback row.
    ///
    /// Shared prune policy (see the primitives above): drop the episode row, then
    /// its playback row (keyed by the same `episode_id`).
    async fn delete_episode(&self, episode_id: i32) -> Result<()> {
        let ids = [episode_id];
        self.delete_episode_rows(&ids).await?;
        self.delete_playback_rows(&ids).await
    }

    /// Delete a podcast's cached row, all of its episode rows, and those episodes' playback rows. Shared prune
    /// policy (see the primitives above): playbacks have no podcast column (keyed by `episode_id`), so resolve
    /// the podcast's episode ids FIRST, prune those playbacks, then the episode rows, then the podcast row
    /// itself.
    async fn delete_podcast(&self, podcast_id: i32) -> Result<()> {
        let episode_ids = self.list_episode_ids_for_podcast(podcast_id).await?;
        self.delete_playback_rows(&episode_ids).await?;
        self.delete_episode_rows(&episode_ids).await?;
        self.delete_podcast_row(podcast_id).await
    }

    // ── Outbox ──────────────────────────────────────────────────────────

    /// Enqueue an operation for later drain to the server.
    async fn enqueue(&self, op: &OutboxOp) -> Result<()>;

    /// Read all pending outbox operations in enqueue order, returning (op_id, op) pairs.
    async fn pending(&self) -> Result<Vec<(u64, OutboxOp)>>;

    /// Read delivery metadata, including rejected entries retained for review.
    async fn journal_entries(&self) -> Result<Vec<(u64, crate::JournalEntry)>>;

    /// Persist retry counts or quarantine a rejected operation in place.
    async fn save_journal_entry(&self, id: u64, entry: &crate::JournalEntry) -> Result<()>;

    /// Acknowledge an outbox operation after successful server delivery.
    async fn ack(&self, op_id: u64) -> Result<()>;

    // NOTE: device downloads (which episodes have bytes on this device) are
    // NOT tracked here — `halogen-webui-media`'s byte store is the ground truth
    // (`MediaStore::list_ids` hydrates `EpisodeState.client_downloads`). A
    // persisted id list drifted from reality by design-accident before.

    // ── Maintenance ─────────────────────────────────────────────────────

    /// Wipe all locally-cached data (podcasts, episodes, playlists, playbacks,
    /// outbox).
    async fn clear(&self) -> Result<()>;
}
