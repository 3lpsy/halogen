use super::tasks::{
    fetch_episode_chapters_task, fetch_podcast_task, fetch_podcasts_task, server_download_poll_task,
};
use super::{Command, InFlight, SyncService, plural, spawn_task};
use crate::reorder::{ReorderKeys, reorder_ids, sort_ids_by};
use chrono::Utc;
use halogen_api::ApiClient;
use halogen_ui_appstate::QueueState;
use halogen_ui_appstate::state::SyncStatus;
use halogen_ui_logging::{debug, error};
use halogen_ui_svc_store::outbox::OutboxOp;
use halogen_wire::{
    DownloadStatus, EpisodeData, OrderDirection, PlaybackData, PlaybackStatus, PlaylistData,
    PlaylistReorderField, PlaylistUpdateData, PodcastConfigData, PodcastConfigUpdateData,
    PodcastData,
};
use std::collections::HashSet;

impl SyncService {
    /// Persist a server-paged batch of episodes and merge them into the in-memory
    /// indices. The worker is the single store writer, so server-paged views (e.g.
    /// `/latest`) hand their freshly-fetched pages here instead of writing the
    /// store themselves. Idempotent: upserts by id, and only appends ids not
    /// already grouped under their podcast.
    pub(super) async fn cache_episodes(&mut self, mut episodes: Vec<EpisodeData>) {
        if episodes.is_empty() {
            return;
        }
        // Episodes whose optimistic played/download facet hasn't been shipped yet:
        // a `MarkPlayed`/`TriggerDownload`/`RemoveServerDownload` outbox op is still
        // queued, so a server revalidation that predates the drain is STALE for that
        // facet and must not overwrite it — in EITHER direction (mark-played and
        // mark-unplayed; download and remove-download both queue an op and flip the
        // local facet optimistically). Mirrors the sticky `chapters` merge below.
        // Reading the outbox here is cheap — the loop drains after every command, so
        // it's near-empty while online (and `cache_episodes` only runs with server
        // data).
        let mut played_pending: HashSet<i32> = HashSet::new();
        let mut download_pending: HashSet<i32> = HashSet::new();
        if let Ok(pending) = self.store.pending().await {
            for (_, op) in &pending {
                match op {
                    OutboxOp::MarkPlayed { episode_id, .. } => {
                        played_pending.insert(*episode_id);
                    }
                    OutboxOp::TriggerDownload { episode_ids }
                    | OutboxOp::RemoveServerDownload { episode_ids } => {
                        download_pending.extend(episode_ids.iter().copied());
                    }
                    _ => {}
                }
            }
        }

        // Chapters are loaded lazily (opt-in include) and the lists don't carry
        // them, so keep an already-loaded set sticky: a chapter-less body (`None`
        // = "not requested") inherits the chapters we already have. `Some(_)`
        // (including an empty set) is authoritative and overwrites. Applied before
        // the store write so the cache and memory agree (chapters survive reload).
        for ep in episodes.iter_mut() {
            let prev_facets = self.app_state.episodes_by_id.get(&ep.id);
            if ep.chapters.is_none()
                && let Some(prev) = prev_facets
                && prev.chapters.is_some()
            {
                ep.chapters = prev.chapters.clone();
            }
            // Don't let a stale revalidation overwrite a locally-set playback or
            // download facet while its outbox op is still queued. The optimistic
            // local value is authoritative in BOTH directions until the op drains, so
            // keep `prev` whenever the incoming server value differs (e.g. a queued
            // mark-unplayed must survive a page that still says Finished; a queued
            // remove-download must survive a page that still says Downloaded).
            if let Some(prev) = prev_facets {
                if played_pending.contains(&ep.id) && ep.playback_status != prev.playback_status {
                    ep.playback_status = prev.playback_status.clone();
                }
                if download_pending.contains(&ep.id) && ep.download_status != prev.download_status {
                    ep.download_status = prev.download_status.clone();
                }
            }
        }
        if let Err(e) = self.store.upsert_episodes(&episodes).await {
            error!(error = %e, "Failed to cache fetched episodes");
        }
        let mut referenced_podcasts: HashSet<i32> = HashSet::new();
        for ep in episodes {
            referenced_podcasts.insert(ep.podcast_id);
            let ids = self
                .app_state
                .episodes_by_podcast
                .entry(ep.podcast_id)
                .or_default();
            if !ids.contains(&ep.id) {
                ids.push(ep.id);
            }
            // A finished/failed server download leaves `Downloading` → drop any
            // in-flight progress ring for it (the poll task also clears this, but a
            // plain pull may observe completion first).
            if ep.download_status != DownloadStatus::Downloading {
                self.downloads.server_download_progress.remove(&ep.id);
            }
            self.app_state.episodes_by_id.insert(ep.id, ep);
        }
        // Prime the pool for this page's podcasts in ONE request rather than the
        // per-row `EnsurePodcast` N+1. Done before `publish` so the in-flight guards
        // are set by the time the re-rendered rows dispatch their own `EnsurePodcast`
        // (which then no-op).
        self.ensure_podcasts_batch(referenced_podcasts);
        self.publish();
    }

    /// Persist a server-paged batch of podcasts and merge them into the pool
    /// (`podcasts_by_id`) for O(1) detail/title lookups. The paged podcasts list
    /// (and on-demand detail fetches) hand pages here.
    pub(super) async fn cache_podcasts(&mut self, podcasts: Vec<PodcastData>) {
        if podcasts.is_empty() {
            return;
        }
        // Drop any podcast the user unsubscribed this session: a list page fetched
        // before the queued `Unsubscribe` reached the server is STALE and must not
        // resurrect the removed podcast in the pool. Without this, navigating to
        // /podcasts right after a delete re-fetches page 0 — which still carries the
        // podcast until the server processes the queued op — and re-adds it to
        // `podcasts_by_id`. The `unsubscribed` tombstone outlives the outbox op, so
        // this holds even when the op drains before this stale page's cache write.
        let podcasts: Vec<PodcastData> = podcasts
            .into_iter()
            .filter(|p| !self.unsubscribed.contains(&p.id))
            .collect();
        if podcasts.is_empty() {
            return;
        }
        if let Err(e) = self.store.upsert_podcasts(&podcasts).await {
            error!(error = %e, "Failed to cache fetched podcasts");
        }
        for p in podcasts {
            self.podcasts.podcasts_by_id.insert(p.id, p);
        }
        self.publish();
    }

    /// Upsert playlists into local state + store after a direct create/update.
    /// Replaces an existing row by id (preserving its already-loaded `episode_ids`
    /// if the incoming row doesn't carry them) or appends a new one. If an incoming
    /// playlist is the new default, clears `is_default` on the others locally so the
    /// UI never shows two defaults before the next `RefreshNow` pull reconciles.
    pub(super) async fn cache_playlists(&mut self, mut playlists: Vec<PlaylistData>) {
        if playlists.is_empty() {
            return;
        }
        // Playlists whose membership facet hasn't been shipped yet: a queued
        // `AddToPlaylist`/`RemoveFromPlaylist`/`MoveInPlaylist`/`ReorderPlaylist`
        // outbox op means the LOCAL membership (already mutated optimistically) is
        // authoritative until the op drains — a server row fetched before the
        // drain is STALE for that facet and must not overwrite it (it would drop a
        // queued add from the queue, or resurrect a queued remove, until the next
        // pull). Mirrors the pending-op facet guard in `cache_episodes`.
        let mut membership_pending: HashSet<i32> = HashSet::new();
        if let Ok(pending) = self.store.pending().await {
            for (_, op) in &pending {
                match op {
                    OutboxOp::AddToPlaylist { playlist_id, .. }
                    | OutboxOp::RemoveFromPlaylist { playlist_id, .. }
                    | OutboxOp::MoveInPlaylist { playlist_id, .. }
                    | OutboxOp::ReorderPlaylist { playlist_id, .. } => {
                        membership_pending.insert(*playlist_id);
                    }
                    _ => {}
                }
            }
        }
        // Applied BEFORE the store write so the cache and memory agree (like the
        // facet guard in `cache_episodes`): a guarded row keeps the local
        // membership in place of the server's.
        for pl in playlists.iter_mut() {
            if pl.episode_ids.is_some()
                && membership_pending.contains(&pl.id)
                && let Some(local) = self.playlists.episodes_by_playlist.get(&pl.id)
            {
                pl.episode_ids = Some(local.clone());
            }
        }
        if let Err(e) = self.store.upsert_playlists(&playlists).await {
            error!(error = %e, "Failed to cache playlists");
        }
        let prev_queue = self.playlists.queue_id();
        let new_default_ids: Vec<i32> = playlists
            .iter()
            .filter(|p| p.is_default)
            .map(|p| p.id)
            .collect();
        if !new_default_ids.is_empty() {
            for existing in self.playlists.playlists.iter_mut() {
                if existing.is_default && !new_default_ids.contains(&existing.id) {
                    existing.is_default = false;
                }
            }
        }
        for pl in playlists {
            // When the row carries its membership (the list page requests
            // `EpisodeIds`), mirror it into the id-list index that the lazy
            // detail/queue path reads (`episodes_by_playlist`) — same invariant
            // `hydrate_from_store` and the default-playlist pull maintain. A row
            // *without* `episode_ids` (an offline edit, a create) leaves any
            // existing membership untouched.
            if let Some(ids) = pl.episode_ids.clone() {
                self.playlists.episodes_by_playlist.insert(pl.id, ids);
            }
            if let Some(slot) = self.playlists.playlists.iter_mut().find(|p| p.id == pl.id) {
                let prev_ids = slot.episode_ids.clone();
                *slot = pl;
                if slot.episode_ids.is_none() {
                    slot.episode_ids = prev_ids;
                }
            } else {
                self.playlists
                    .episodes_by_playlist
                    .entry(pl.id)
                    .or_default();
                self.playlists.playlists.push(pl);
            }
        }
        // Re-resolve the queue. A new default → `Present`; if this batch unset the
        // playlist that WAS the queue (e.g. an offline edit unchecking "make
        // default"), it may now be `Absent`. Only assert `Absent` in those cases —
        // caching an unrelated playlist must not flip a known queue to absent.
        let queue_unset = prev_queue.is_some_and(|qid| {
            self.playlists
                .playlists
                .iter()
                .any(|p| p.id == qid && !p.is_default)
        });
        self.playlists
            .recompute_queue(!new_default_ids.is_empty() || queue_unset);
        self.publish();
    }

    /// Apply an offline playlist edit to local state. Mirrors the server's update
    /// semantics: a field set to `Some` is applied, `None` leaves it unchanged
    /// (so, like the server, an empty description can't clear an existing one).
    /// Routes through `cache_playlists`, which handles the queue/default bookkeeping.
    pub(super) async fn update_playlist_locally(
        &mut self,
        playlist_id: i32,
        data: &PlaylistUpdateData,
    ) {
        let Some(mut pl) = self
            .playlists
            .playlists
            .iter()
            .find(|p| p.id == playlist_id)
            .cloned()
        else {
            return;
        };
        data.apply_to(&mut pl);
        self.cache_playlists(vec![pl]).await;
    }

    /// Apply an offline podcast-config edit to the cached podcast. Mirrors the
    /// server's update semantics: a field set to `Some` is applied, `None` leaves
    /// it unchanged. The form always sends a full set, so this is effectively a
    /// replace; `None` handling keeps parity with the server regardless. Routes
    /// through `cache_podcasts`, the single podcast-pool writer.
    pub(super) async fn update_podcast_config_locally(
        &mut self,
        podcast_id: i32,
        config_id: i32,
        data: &PodcastConfigUpdateData,
    ) {
        let Some(mut pod) = self.podcasts.podcast(podcast_id).cloned() else {
            return;
        };
        let mut cfg = pod.podcast_config.clone().unwrap_or(PodcastConfigData {
            id: config_id,
            poll_interval_seconds: None,
            max_episodes: None,
            max_concurrent_downloads: None,
            auto_download_enabled: None,
            created_at: pod.created_at,
            updated_at: pod.updated_at,
        });
        data.apply_to(&mut cfg);
        pod.podcast_config_id = Some(cfg.id);
        pod.podcast_config = Some(cfg);
        self.cache_podcasts(vec![pod]).await;
    }

    /// Apply an offline podcast-config removal to the cached podcast: drop the
    /// config + its FK so the UI reverts to the global-defaults state. Routes
    /// through `cache_podcasts`.
    pub(super) async fn remove_podcast_config_locally(&mut self, podcast_id: i32) {
        let Some(mut pod) = self.podcasts.podcast(podcast_id).cloned() else {
            return;
        };
        pod.podcast_config_id = None;
        pod.podcast_config = None;
        self.cache_podcasts(vec![pod]).await;
    }

    /// Resolve the queue (default playlist) via the targeted endpoint and update
    /// `EpisodeState.queue`. Upserts the returned playlist (so its episode ids are known
    /// even if it's outside a paged list). Leaves the queue untouched on a transient
    /// fetch failure (offline) — `Unknown`/stale is safer than a wrong `Absent`.
    pub(super) async fn ensure_default_playlist(&mut self) {
        // Authoritative when online: ask the server for the user's default playlist.
        // Pre-auth / offline / on error, fall back to the locally-cached default so
        // the queue still resolves from local storage — only a successful server
        // answer of `None` can assert the queue is genuinely Absent.
        if self.pull_in_flight || self.recently_pulled() {
            // A pull is already fetching the default playlist, OR one just completed
            // (the queue page mounts right after the SetAuth/WsConnected pulls fire at
            // startup — sometimes a beat after the first finishes, so the in-flight
            // guard alone misses it). Reuse that refresh instead of a third redundant
            // GET; resolve the queue from cache — the pull published / will publish
            // the authoritative result.
            self.playlists.recompute_queue(false);
            self.publish();
            return;
        }
        if self.manual_offline {
            self.playlists.recompute_queue(false);
            self.publish();
            return;
        }
        let Some(api) = self.api_client.as_ref().map(|a| a.clone_handle()) else {
            self.playlists.recompute_queue(false);
            self.publish();
            return;
        };
        match api.get_default_playlist().await {
            Ok(Some(pl)) => self.cache_playlists(vec![pl]).await,
            Ok(None) => {
                self.playlists.queue = QueueState::Absent;
                self.publish();
            }
            Err(e) => {
                debug!(error = %e, "ensure_default_playlist deferred (offline?) — using local queue");
                self.playlists.recompute_queue(false);
                self.publish();
            }
        }
    }

    /// The in-flight dedup set for a [`Self::spawn_deduped`] kind.
    pub(super) fn in_flight_set(&mut self, kind: InFlight) -> &mut HashSet<i32> {
        match kind {
            InFlight::Podcasts => &mut self.podcasts_in_flight,
            InFlight::EpisodeChapters => &mut self.episode_chapters_in_flight,
            InFlight::ServerPolls => &mut self.server_polls_in_flight,
        }
    }

    /// Spawn a de-duped detached task keyed on `(kind, key)`: a no-op when manually
    /// offline, when `key` is already in flight for `kind`, or when there's no API
    /// client yet (dropping the guard so a later call retries). Otherwise builds the
    /// task with a fresh API handle + the internal channel and spawns it, dropping
    /// the guard if the spawn fails. The single shape behind every `ensure_*`.
    pub(super) fn spawn_deduped<Fut>(
        &mut self,
        kind: InFlight,
        key: i32,
        make_task: impl FnOnce(i32, ApiClient, futures::channel::mpsc::UnboundedSender<Command>) -> Fut,
    ) where
        Fut: std::future::Future<Output = ()> + 'static,
    {
        if self.manual_offline {
            return;
        }
        if !self.in_flight_set(kind).insert(key) {
            return; // already in flight
        }
        let (Some(tx), Some(api)) = (
            self.internal_tx.clone(),
            self.api_client.as_ref().map(|a| a.clone_handle()),
        ) else {
            // Offline / pre-auth: drop the guard so it retries once a client exists.
            self.in_flight_set(kind).remove(&key);
            return;
        };
        if !spawn_task(make_task(key, api, tx)) {
            self.in_flight_set(kind).remove(&key);
        }
    }

    /// Lazily load a podcast into the pool for title/art lookups (episode lists no
    /// longer carry a nested podcast join). De-duped; a no-op when already cached.
    pub(super) fn ensure_podcast(&mut self, podcast_id: i32) {
        if self.podcasts.podcasts_by_id.contains_key(&podcast_id) {
            return;
        }
        self.spawn_deduped(InFlight::Podcasts, podcast_id, fetch_podcast_task);
    }

    /// Batch-prime the pool for a freshly-cached episode page: fetch every
    /// referenced podcast that isn't already cached or in flight in ONE
    /// `list_podcasts(filter.ids=…)` request, instead of N per-row `ensure_podcast`
    /// round trips. Shares the `podcasts_in_flight` guard set with `ensure_podcast`,
    /// so the two never double-fetch the same id: marking the batch's ids in flight
    /// here makes concurrent per-row `EnsurePodcast` dispatches no-op, and a per-row
    /// fetch already running keeps that id out of the batch.
    pub(super) fn ensure_podcasts_batch(&mut self, podcast_ids: HashSet<i32>) {
        if self.manual_offline {
            return;
        }
        let missing: Vec<i32> = podcast_ids
            .into_iter()
            .filter(|id| {
                !self.podcasts.podcasts_by_id.contains_key(id)
                    && !self.podcasts_in_flight.contains(id)
            })
            .collect();
        if missing.is_empty() {
            return;
        }
        let (Some(tx), Some(api)) = (
            self.internal_tx.clone(),
            self.api_client.as_ref().map(|a| a.clone_handle()),
        ) else {
            // Offline / pre-auth: leave the ids unguarded so a later page (or the
            // per-row `ensure_podcast` fallback) retries once a client exists.
            return;
        };
        for id in &missing {
            self.podcasts_in_flight.insert(*id);
        }
        if !spawn_task(fetch_podcasts_task(missing.clone(), api, tx)) {
            for id in &missing {
                self.podcasts_in_flight.remove(id);
            }
        }
    }

    /// Lazily load an episode's chapter markers for the now-playing display.
    /// Chapters are an opt-in include the episode lists don't carry, so the player
    /// asks for them on demand. De-duped; a no-op when already loaded (`Some(_)`,
    /// including an empty set).
    pub(super) fn ensure_episode_chapters(&mut self, episode_id: i32) {
        if self
            .app_state
            .episode(episode_id)
            .is_some_and(|e| e.chapters.is_some())
        {
            return; // already loaded (Some([]) means "loaded, none exist")
        }
        self.spawn_deduped(
            InFlight::EpisodeChapters,
            episode_id,
            fetch_episode_chapters_task,
        );
    }

    /// Spawn a ≈1 Hz poller that mirrors the server's download progress for one
    /// episode into `server_download_progress` (a filling ring on the download
    /// controls), then refreshes the durable status when it finishes. De-duped.
    pub(super) fn ensure_server_progress_poll(&mut self, episode_id: i32) {
        let offline = self.offline_flag.clone();
        self.spawn_deduped(InFlight::ServerPolls, episode_id, move |id, api, tx| {
            server_download_poll_task(id, api, tx, offline)
        });
    }

    /// Ack (drop) every still-pending `SetCursor` op for `episode_id`. Called
    /// before enqueueing a fresh cursor write (`SetCursor` / `MarkPlayed`, which
    /// also writes the cursor): the server's cursor is last-write-wins per
    /// episode, so the superseded intermediates are pure waste — a long offline
    /// listening session queued one per ~10s save and reconnect then drained
    /// them one serial HTTP round-trip at a time, stalling everything behind
    /// them. Safe from races: the worker loop is a single cooperative task, so
    /// no drain is mid-flight while a command is being handled.
    pub(super) async fn coalesce_pending_cursor(&mut self, episode_id: i32) {
        let Ok(pending) = self.store.pending().await else {
            return;
        };
        for (op_id, op) in pending {
            if matches!(op, OutboxOp::SetCursor { episode_id: e, .. } if e == episode_id)
                && let Err(e) = self.store.ack(op_id).await
            {
                error!(error = %e, op_id, "Failed to drop a superseded SetCursor op");
            }
        }
    }

    /// Enqueue an outbox op, logging (but not surfacing) a local-store failure.
    pub(super) async fn enqueue(&mut self, op: OutboxOp) {
        if let Err(e) = self.store.enqueue(&op).await {
            error!(error = %e, op = ?op, "Failed to enqueue outbox op");
            self.connection.last_error = Some(format!("Could not save action: {e}"));
            self.toasts
                .error("Couldn't save your action on this device.");
        }
    }

    // ── Optimistic local mutations ──────────────────────────────────────

    /// Insert-or-update the playback row for an episode, apply `f`, then persist it
    /// to the local store. Write-through (like `set_download_status_locally`) is
    /// required now that playbacks aren't bulk-pulled: this optimistic cursor is the
    /// only durable record until the outbox `SetCursor`/`MarkPlayed` reaches the
    /// server, so it must survive a reload — especially offline (boot
    /// `hydrate_from_store` rebuilds the overlay + History from these rows).
    pub(super) async fn set_playback_locally(
        &mut self,
        episode_id: i32,
        f: impl FnOnce(&mut PlaybackData),
    ) {
        let now = Utc::now();
        let pb = {
            let pb = self
                .playbacks
                .playbacks
                .entry(episode_id)
                .or_insert_with(|| PlaybackData {
                    id: 0,
                    user_id: 0,
                    episode_id,
                    cursor: 0,
                    completed: false,
                    created_at: now,
                    updated_at: now,
                });
            f(pb);
            pb.updated_at = now;
            pb.clone()
        };
        if let Err(e) = self.store.save_playback(&pb).await {
            error!(episode_id, error = %e, "Failed to persist playback locally");
        }
    }

    /// Optimistically mutate a cached episode and write it through to the store.
    /// Paged lists (e.g. /latest) render rows from the store, so an in-memory-only
    /// change wouldn't survive a re-read/reload — the badge/filters would snap back.
    /// No-op for an episode that isn't cached. `what` labels the persist-failure log.
    pub(super) async fn mutate_episode_locally(
        &mut self,
        episode_id: i32,
        what: &str,
        f: impl FnOnce(&mut EpisodeData),
    ) {
        let Some(ep) = self.app_state.episodes_by_id.get_mut(&episode_id) else {
            return;
        };
        f(ep);
        let ep = ep.clone();
        if let Err(e) = self.store.upsert_episodes(&[ep]).await {
            error!(episode_id, what, error = %e, "Failed to persist episode change");
        }
    }

    /// Optimistically set an episode's server `download_status` so the UI's
    /// download icon updates immediately (reconciled on the next pull).
    pub(super) async fn set_download_status_locally(
        &mut self,
        episode_id: i32,
        status: DownloadStatus,
    ) {
        self.mutate_episode_locally(episode_id, "download status", |ep| {
            ep.download_status = status;
        })
        .await;
    }

    /// Optimistically set an episode's per-user `playback_status`. The playbacks
    /// overlay (cursor/completed) is the durable record, but paged lists filter on
    /// the cached `EpisodeData.playback_status` facet, so it must be kept in
    /// lock-step or a "Finished"/"Unplayed" filter desyncs until the next pull.
    pub(super) async fn set_playback_status_locally(
        &mut self,
        episode_id: i32,
        status: PlaybackStatus,
    ) {
        self.mutate_episode_locally(episode_id, "playback status", |ep| {
            ep.playback_status = status;
        })
        .await;
    }

    /// Drop an episode's device download: forget the client state and delete the
    /// stored bytes. Shared by the single `RemoveDownload` and the bulk device ops.
    pub(super) async fn remove_device_download(&mut self, episode_id: i32) {
        self.downloads.client_downloads.remove(&episode_id);
        self.downloads.download_progress.remove(&episode_id);
        if let Some(media) = self.media.as_ref()
            && let Err(e) = media.remove_audio(episode_id).await
        {
            error!(episode_id, error = %e, "Failed to delete device audio");
        }
    }

    /// Whether the worker currently believes it's offline (a device download can't
    /// pull the server's copy, so bulk device ops short-circuit on this).
    pub(super) fn is_offline(&self) -> bool {
        matches!(self.connection.sync_status, SyncStatus::Offline)
    }

    /// One summary toast for a multiselect batch action, shown only when `n > 1` — a
    /// single-row action stays quiet (its spinner / progress ring is the feedback).
    /// `msg` receives the count and the matching plural suffix so each call site keeps
    /// its own wording. `bulk_info`/`bulk_success` pick the toast level.
    pub(super) fn bulk_info(&self, n: usize, msg: impl FnOnce(usize, &str) -> String) {
        if n > 1 {
            self.toasts.info(msg(n, plural(n)));
        }
    }

    pub(super) fn bulk_success(&self, n: usize, msg: impl FnOnce(usize, &str) -> String) {
        if n > 1 {
            self.toasts.success(msg(n, plural(n)));
        }
    }

    /// "queue" when `playlist_id` is the default playlist, else "playlist" — the
    /// noun the bulk add/remove summary toasts use.
    pub(super) fn playlist_dest(&self, playlist_id: i32) -> &'static str {
        if self.playlists.queue_id() == Some(playlist_id) {
            "queue"
        } else {
            "playlist"
        }
    }

    /// Append an episode id to a playlist's membership if not already present.
    /// Insert an episode into a playlist's in-memory membership at `position`
    /// (`Some(i)` inserts at the clamped index, `None` appends). Mirrors the
    /// server's insert so a FIFO outbox replay reproduces this order. No-op if the
    /// episode is already a member.
    pub(super) fn add_to_playlist_locally(
        &mut self,
        playlist_id: i32,
        episode_id: i32,
        position: Option<i32>,
    ) {
        let ids = self
            .playlists
            .episodes_by_playlist
            .entry(playlist_id)
            .or_default();
        if !ids.contains(&episode_id) {
            match position {
                Some(p) => {
                    let at = (p.max(0) as usize).min(ids.len());
                    ids.insert(at, episode_id);
                }
                None => ids.push(episode_id),
            }
        }
    }

    /// Reorder an episode within a playlist's in-memory membership Vec: pull it
    /// out and re-insert at `to` (clamped). No-op if absent or unchanged. The Vec
    /// IS the canonical client order; `persist_playlist` writes it through. The
    /// clamp matches the server's so a FIFO outbox replay reproduces this order.
    pub(super) fn move_in_playlist_locally(&mut self, playlist_id: i32, episode_id: i32, to: i32) {
        if let Some(ids) = self.playlists.episodes_by_playlist.get_mut(&playlist_id) {
            reorder_ids(ids, episode_id, to);
        }
    }

    /// Optimistically reorder a playlist's in-memory membership by `field`/
    /// `direction`, resolving each id's sort keys from the episode pool (see
    /// [`sort_ids_by`]). The server is authoritative on flush; this just gives
    /// instant feedback. `persist_playlist` writes the new order through.
    pub(super) fn reorder_playlist_locally(
        &mut self,
        playlist_id: i32,
        field: PlaylistReorderField,
        direction: OrderDirection,
    ) {
        let Some(mut ids) = self
            .playlists
            .episodes_by_playlist
            .get(&playlist_id)
            .cloned()
        else {
            return;
        };
        sort_ids_by(&mut ids, field, direction, |id| {
            self.app_state.episode(id).map(|e| ReorderKeys {
                published_at: e.published_at,
                title: e.title.clone(),
                duration_secs: e.duration_secs,
                created_at: e.created_at,
            })
        });
        self.playlists.episodes_by_playlist.insert(playlist_id, ids);
    }

    /// Persist a playlist's current membership (from `episodes_by_playlist`) onto
    /// its stored `PlaylistData.episode_ids`, so an optimistic add/remove survives
    /// offline until the next pull reconciles it.
    pub(super) async fn persist_playlist(&mut self, playlist_id: i32) {
        let ids = self
            .playlists
            .episodes_by_playlist
            .get(&playlist_id)
            .cloned()
            .unwrap_or_default();
        let updated = self
            .playlists
            .playlists
            .iter_mut()
            .find(|p| p.id == playlist_id)
            .map(|pl| {
                pl.episode_ids = Some(ids);
                pl.clone()
            });
        if let Some(pl) = updated
            && let Err(e) = self.store.upsert_playlists(&[pl]).await
        {
            error!(error = %e, "Failed to persist playlist membership");
        }
    }

    /// Optimistically reorder a playlist within the user's manual order: rewrite
    /// `position` across the loaded pool to the new contiguous order and persist the
    /// changed rows. Operates over the LOADED subset (lazy paging means the full set
    /// may not be in memory) — the server rewrites the full 0..n on drain and the
    /// next pull reconciles. No-op if the playlist isn't loaded or `to` is unchanged.
    pub(super) async fn move_playlist_locally(&mut self, playlist_id: i32, to: i32) {
        // Current manual order over the loaded pool (position, id tiebreak).
        let mut order: Vec<i32> = {
            let mut pls: Vec<&PlaylistData> = self.playlists.playlists.iter().collect();
            pls.sort_by(|a, b| a.position.cmp(&b.position).then(a.id.cmp(&b.id)));
            pls.into_iter().map(|p| p.id).collect()
        };
        let before = order.clone();
        reorder_ids(&mut order, playlist_id, to);
        if order == before {
            // Unchanged, absent, or clamped-to-same → nothing to persist.
            return;
        }

        let mut changed: Vec<PlaylistData> = Vec::new();
        for (new_pos, pid) in order.iter().enumerate() {
            if let Some(pl) = self.playlists.playlists.iter_mut().find(|p| p.id == *pid)
                && pl.position != new_pos as i32
            {
                pl.position = new_pos as i32;
                changed.push(pl.clone());
            }
        }
        if !changed.is_empty()
            && let Err(e) = self.store.upsert_playlists(&changed).await
        {
            error!(error = %e, "Failed to persist playlist reorder");
        }
    }

    /// Drop a playlist from local state + the store after a confirmed server-side
    /// delete: the pool row, its `episodes_by_playlist` membership, and the cached
    /// store row (whose `episode_ids` carry the membership — there is no separate
    /// cached episode list to prune). Recomputes the queue, allowing `Absent` only
    /// when the deleted playlist WAS the queue — the one case where we positively
    /// know the default is gone (a partial pool otherwise must not assert absence).
    pub(super) async fn remove_playlist_locally(&mut self, playlist_id: i32) {
        let was_queue = self.playlists.queue_id() == Some(playlist_id);
        self.playlists.playlists.retain(|p| p.id != playlist_id);
        self.playlists.episodes_by_playlist.remove(&playlist_id);
        self.playlists.recompute_queue(was_queue);
        if let Err(e) = self.store.delete_playlist_row(playlist_id).await {
            error!(playlist_id, error = %e, "Failed to delete cached playlist row");
        }
    }

    /// Drop a podcast and its episodes from local state (optimistic unsubscribe).
    pub(super) fn remove_podcast_locally(&mut self, podcast_id: i32) {
        self.podcasts.podcasts_by_id.remove(&podcast_id);
        if let Some(ids) = self.app_state.episodes_by_podcast.remove(&podcast_id) {
            for id in ids {
                self.app_state.episodes_by_id.remove(&id);
            }
        }
    }

    /// Drop a set of episode ids from every local playlist's in-memory membership,
    /// persisting only the playlists that actually changed. Shared by the
    /// `RemoveLocal*` prune handlers (an episode is one id; a podcast is many).
    pub(super) async fn prune_episodes_from_playlists(&mut self, ids: &HashSet<i32>) {
        let affected: Vec<i32> = self
            .playlists
            .episodes_by_playlist
            .iter_mut()
            .filter_map(|(playlist_id, members)| {
                let before = members.len();
                members.retain(|id| !ids.contains(id));
                (members.len() != before).then_some(*playlist_id)
            })
            .collect();
        for playlist_id in affected {
            self.persist_playlist(playlist_id).await;
        }
    }

    /// Remove all locally-cached data for one episode WITHOUT touching the server
    /// (see [`Command::RemoveLocalEpisodeData`]): the device audio blob, cached
    /// metadata, playback position, download state, and playlist/queue membership.
    /// Purely local recovery — no outbox op — so the episode re-syncs on the next
    /// pull. The outer loop publishes the resulting state.
    pub(super) async fn remove_local_episode_data(&mut self, episode_id: i32) {
        // Device audio bytes + client download state (client_downloads /
        // download_progress).
        self.remove_device_download(episode_id).await;

        // Cached metadata row + its playback row.
        if let Err(e) = self.store.delete_episode(episode_id).await {
            error!(episode_id, error = %e, "Failed to delete local episode data");
        }

        // In-memory pool: drop the episode, its grouping under the podcast, and its
        // playback overlay. The removed body carries the podcast id for the regroup.
        if let Some(ep) = self.app_state.episodes_by_id.remove(&episode_id)
            && let Some(ids) = self.app_state.episodes_by_podcast.get_mut(&ep.podcast_id)
        {
            ids.retain(|id| *id != episode_id);
        }
        self.playbacks.playbacks.remove(&episode_id);

        // Playlist/queue membership.
        self.prune_episodes_from_playlists(&HashSet::from([episode_id]))
            .await;

        self.toasts.success("Removed this episode's local data");
    }

    /// Remove all locally-cached data for a podcast and every one of its episodes
    /// WITHOUT unsubscribing on the server (see [`Command::RemoveLocalPodcastData`],
    /// unlike [`Command::Unsubscribe`]): device audio, cached metadata, playbacks,
    /// download state, and playlist/queue membership. Purely local recovery — no
    /// outbox op — so the podcast re-syncs on the next pull.
    pub(super) async fn remove_local_podcast_data(&mut self, podcast_id: i32) {
        // Resolve the FULL episode set from the store, not just the in-memory pool:
        // lazy paging may have left some of this podcast's episodes on disk but not
        // in memory, and they all need their audio/playbacks/membership pruned.
        let episode_ids: HashSet<i32> = match self.store.list_episodes(podcast_id).await {
            Ok(eps) => eps.into_iter().map(|e| e.id).collect(),
            Err(e) => {
                error!(podcast_id, error = %e, "Failed to list episodes for local prune");
                // Fall back to the in-memory grouping so we still prune what we know.
                self.app_state
                    .episodes_by_podcast
                    .get(&podcast_id)
                    .map(|ids| ids.iter().copied().collect())
                    .unwrap_or_default()
            }
        };

        // Device audio bytes + client download state for each episode.
        for id in &episode_ids {
            self.remove_device_download(*id).await;
        }

        // Cached podcast row + its episode rows + those episodes' playback rows.
        if let Err(e) = self.store.delete_podcast(podcast_id).await {
            error!(podcast_id, error = %e, "Failed to delete local podcast data");
        }

        // In-memory pool: podcast + its episodes (by_id / by_podcast), then the
        // playback overlay for each episode.
        self.remove_podcast_locally(podcast_id);
        for id in &episode_ids {
            self.playbacks.playbacks.remove(id);
        }

        // Playlist/queue membership for every removed episode.
        self.prune_episodes_from_playlists(&episode_ids).await;

        self.toasts.success("Removed this podcast's local data");
    }
}

#[cfg(test)]
mod cache_playlists_tests {
    use std::rc::Rc;

    use futures::executor::block_on;
    use halogen_ui_svc_store::NativeLocalStore;

    use super::*;
    use crate::SyncService;

    /// A service over a fresh throwaway SQLite store (no media, no api client).
    /// The events receiver is dropped — `publish` tolerates a closed channel.
    fn test_service(db: &str) -> SyncService {
        let path =
            std::env::temp_dir().join(format!("halogen-sync-test-{}-{db}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let store = Rc::new(NativeLocalStore::open(path).expect("open test store"));
        let (tx, _rx) = futures::channel::mpsc::unbounded();
        SyncService::new(store, None, tx)
    }

    fn playlist(id: i32, episode_ids: Option<Vec<i32>>) -> PlaylistData {
        let now = Utc::now();
        PlaylistData {
            id,
            name: format!("Playlist {id}"),
            description: None,
            is_default: false,
            position: 0,
            on_remove_delete_file_server: false,
            on_remove_delete_file_client: false,
            created_at: now,
            updated_at: now,
            episode_ids,
            episode_playlist: None,
        }
    }

    #[test]
    fn stale_pull_keeps_queued_playlist_membership() {
        let mut svc = test_service("playlist-guard");
        block_on(async {
            // Optimistic local add of episode 42 (what the AddToPlaylist command
            // handler does): membership mutated + the op queued, not yet drained.
            svc.playlists.playlists.push(playlist(1, Some(vec![7, 42])));
            svc.playlists.episodes_by_playlist.insert(1, vec![7, 42]);
            svc.enqueue(OutboxOp::AddToPlaylist {
                playlist_id: 1,
                episode_ids: vec![42],
                position: None,
            })
            .await;

            // A server page fetched BEFORE the drain is stale: it still lacks 42.
            // The queued op guards the membership facet — 42 must survive.
            svc.cache_playlists(vec![playlist(1, Some(vec![7]))]).await;
            assert_eq!(
                svc.playlists.episodes_by_playlist.get(&1),
                Some(&vec![7, 42]),
                "a stale pull must not drop the queued AddToPlaylist episode"
            );
            // The persisted row agrees with memory (the guard runs before the
            // store write).
            let stored = svc.store.list_playlists().await.unwrap();
            assert_eq!(stored[0].episode_ids, Some(vec![7, 42]));

            // Once the op drains (acked), the server list is authoritative again.
            for (op_id, _) in svc.store.pending().await.unwrap() {
                svc.store.ack(op_id).await.unwrap();
            }
            svc.cache_playlists(vec![playlist(1, Some(vec![7]))]).await;
            assert_eq!(svc.playlists.episodes_by_playlist.get(&1), Some(&vec![7]));
        });
    }

    #[test]
    fn stale_pull_does_not_resurrect_queued_playlist_remove() {
        let mut svc = test_service("playlist-guard-remove");
        block_on(async {
            // Optimistic local remove of episode 7 with the op still queued.
            svc.playlists.playlists.push(playlist(2, Some(vec![9])));
            svc.playlists.episodes_by_playlist.insert(2, vec![9]);
            svc.enqueue(OutboxOp::RemoveFromPlaylist {
                playlist_id: 2,
                episode_ids: vec![7],
            })
            .await;

            // The stale server page still carries the removed episode.
            svc.cache_playlists(vec![playlist(2, Some(vec![9, 7]))])
                .await;
            assert_eq!(
                svc.playlists.episodes_by_playlist.get(&2),
                Some(&vec![9]),
                "a stale pull must not resurrect the queued RemoveFromPlaylist episode"
            );
        });
    }

    #[test]
    fn pull_without_pending_ops_overwrites_membership() {
        let mut svc = test_service("playlist-noguard");
        block_on(async {
            // Nothing queued for this playlist — the server list wins (an op on a
            // DIFFERENT playlist must not guard it either).
            svc.playlists.playlists.push(playlist(3, Some(vec![1, 2])));
            svc.playlists.episodes_by_playlist.insert(3, vec![1, 2]);
            svc.enqueue(OutboxOp::AddToPlaylist {
                playlist_id: 99,
                episode_ids: vec![5],
                position: None,
            })
            .await;

            svc.cache_playlists(vec![playlist(3, Some(vec![2, 1, 4]))])
                .await;
            assert_eq!(
                svc.playlists.episodes_by_playlist.get(&3),
                Some(&vec![2, 1, 4])
            );
        });
    }
}
