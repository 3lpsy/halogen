//! The `LocalStore` persistence trait + the `StoreHandle` context handle.
//! Per-target backends live in `native` (SQLite) / `web` (localStorage); the
//! query model + outbox are in `query`/`outbox`.

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

/// Platform-agnostic persistence layer.
///
/// Implemented per-target:
/// - Native: `rusqlite` at the platform app-data dir.
/// - Web: IndexedDB, one record per row (`halogen.store.{segment}`, via
///   `halogen-ui-idb`). The AUDIO byte store in `halogen-ui-svc-media` is a
///   separate IndexedDB database — metadata and bytes are separate stores by
///   design.
#[async_trait(?Send)]
pub trait LocalStore {
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

    // ── Local prune ─────────────────────────────────────────────────────
    //
    // Targeted, local-only deletes backing the "Remove local data" recovery
    // action (see `Command::RemoveLocal*`). They drop cached metadata + playback
    // ONLY — the device audio blob (`halogen-ui-svc-media`) and in-memory playlist
    // membership are pruned by the worker, not here. Nothing reaches the server;
    // the rows re-populate on the next pull.
    //
    // The PRUNE POLICY (what gets dropped, and in what order) lives ONCE, in the
    // `delete_episode` / `delete_podcast` DEFAULT methods below — written purely in
    // terms of the storage PRIMITIVES each backend supplies (`list_episode_ids_for_
    // podcast`, `delete_episode_rows`, `delete_playback_rows`, `delete_podcast_row`).
    // Keeping the policy in one place means the native (SQL) and web (in-memory)
    // backends can't drift apart on it; they only differ in how the primitives touch
    // their storage.

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

    /// Delete a podcast's cached row, all of its episode rows, and those
    /// episodes' playback rows.
    ///
    /// Shared prune policy (see the primitives above): playbacks have no podcast
    /// column (keyed by `episode_id`), so resolve the podcast's episode ids FIRST,
    /// prune those playbacks, then the episode rows, then the podcast row itself.
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

    /// Acknowledge an outbox operation after successful server delivery.
    async fn ack(&self, op_id: u64) -> Result<()>;

    // NOTE: device downloads (which episodes have bytes on this device) are
    // NOT tracked here — `halogen-ui-svc-media`'s byte store is the ground truth
    // (`MediaStore::list_ids` hydrates `EpisodeState.client_downloads`). A
    // persisted id list drifted from reality by design-accident before.

    // ── Maintenance ─────────────────────────────────────────────────────

    /// Wipe all locally-cached data (podcasts, episodes, playlists, playbacks,
    /// outbox).
    async fn clear(&self) -> Result<()>;
}
