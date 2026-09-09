//! The sync worker: `SyncService` (owns the canonical `EpisodeState`, the periodic pull/drain loop + command handling)
//! and its worker-internal helpers (`sleep_secs`/`spawn_task`, `InFlight`, the tuning consts). The rest of the inherent
//! impl is split across the sibling modules (connectivity/data_ops/ network); fields are `pub(crate)` so those impls
//! reach them.

use crate::Command;
use crate::runtime::{WorkerEvent, WorkerToasts};
use crate::tracked::Tracked;
use anyhow::Result;
use futures::StreamExt;
use futures::channel::mpsc::UnboundedSender;
use futures::channel::oneshot;
use futures::future::{Either, select};
use halogen_apiclient::ApiClient;
use halogen_webui_app_state::state::{ClientDownloadState, EpisodeState};
use halogen_webui_app_state::{
    ConnectionState, DownloadState, HistoryState, PlaybackState, PlaylistState, PodcastState,
    SessionState,
};
use halogen_webui_component_toast::{ToastPolicy, classify};
use halogen_webui_logging::{debug, error, info, warn};
use halogen_webui_media::MediaStore;
use halogen_webui_store::LocalStore;
use halogen_webui_store::outbox::OutboxOp;
use halogen_wire::{DownloadStatus, EpisodeData, PlaybackStatus};
use std::collections::{HashMap, HashSet};

/// How often the worker pulls fresh data from the server (seconds).
const PULL_INTERVAL_SECS: u64 = 60;

/// Smoothed round-trip latency (ms) above which the connection is "degraded"
/// (yellow) rather than "online" (green). Still online — just sluggish.
pub(crate) const DEGRADED_RTT_MS: u32 = 1000;

/// How many recent pong RTTs to average for the degraded decision (smooths a lone
/// slow ping so the indicator doesn't flicker).
pub(crate) const RTT_WINDOW: usize = 3;

/// Page size for paged `/playbacks` fetches (the History source — see
/// [`SyncService::load_history`]). Nothing is bulk-pulled in the periodic loop any
/// more; episodes/podcasts/playlists/playbacks are all offline-first/paged. The
/// server defaults to size 10 when no pagination is sent, so we always send it.
pub(crate) const PULL_PAGE_SIZE: i32 = 200;

/// Cross-target sleep used to drive the periodic pull.
pub(crate) async fn sleep_secs(secs: u64) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        tokio::time::sleep(std::time::Duration::from_secs(secs)).await;
    }
    #[cfg(target_arch = "wasm32")]
    {
        gloo_timers::future::TimeoutFuture::new((secs * 1000) as u32).await;
    }
}

/// Run !Send downloads outside the sequential worker and report through commands. Native Dioxus uses root-scope
/// spawn_forever because desktop has no LocalSet; renderless tests use Tokio spawn_local inside their LocalSet. Return
/// false without either host so callers report failure instead of staying Downloading.
pub(crate) fn spawn_task(fut: impl std::future::Future<Output = ()> + 'static) -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        wasm_bindgen_futures::spawn_local(fut);
        true
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        if dioxus::core::Runtime::try_current().is_some() {
            dioxus::core::spawn_forever(fut);
            return true;
        }
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            tokio::task::spawn_local(fut)
        }))
        .is_ok()
    }
}

/// Pluralizing suffix for count-based toasts ("1 episode" vs "3 episodes").
pub(crate) fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

/// Which in-flight dedup set a [`SyncService::spawn_deduped`] call keys against.
#[derive(Clone, Copy)]
pub(crate) enum InFlight {
    Podcasts,
    EpisodeChapters,
    ServerPolls,
}

/// Own canonical working state, optimistic commands with durable outbox writes, periodic synchronization, and
/// queued-operation draining. Entity pages load lazily into caches; only this worker publishes domain state to the UI.
pub struct SyncService {
    pub(crate) store: std::rc::Rc<dyn LocalStore>,
    pub(crate) deferred_audio_removals: Option<Vec<i32>>,
    pub(crate) deferred_server_polls: Option<Vec<i32>>,
    pub(crate) deferred_device_starts: Option<Vec<(i32, bool)>>,
    /// Byte storage for device-downloaded audio. `None` = backend failed to
    /// open; device downloads then fail fast as `ClientDownloadState::Failed`.
    pub(crate) media: Option<std::rc::Rc<dyn MediaStore>>,
    pub(crate) app_state: Tracked<EpisodeState>,
    /// Podcast pool + per-podcast auto-playlist config. Its own signal, separate
    /// from `EpisodeState`, so a podcast cache write (paged list / detail fetch) or
    /// an auto-playlist edit only re-renders podcast consumers, not every
    /// `EpisodeState` subscriber.
    pub(crate) podcasts: Tracked<PodcastState>,
    /// Playlists + queue resolution. Its own signal, separate from `EpisodeState`, so
    /// a playlist mutation (add/remove/reorder, default resolution) only re-renders
    /// playlist/queue consumers, not every `EpisodeState` subscriber.
    pub(crate) playlists: Tracked<PlaylistState>,
    /// Playback cursors overlay. Its own signal, separate from `EpisodeState`, so a
    /// cursor save (seek / mark-played) or a History page only re-renders playback
    /// consumers (progress bars, played markers, History), not every `EpisodeState`
    /// subscriber.
    pub(crate) playbacks: Tracked<PlaybackState>,
    /// History `/playbacks` paging cursor. Its own signal, separate from
    /// `EpisodeState`, so advancing it as History pages only re-renders the History
    /// page-ahead effect, not every `EpisodeState` subscriber.
    pub(crate) history: Tracked<HistoryState>,
    /// Device/server download state. Its own signal, separate from `EpisodeState`,
    /// so a ~1 Hz download-progress byte only re-renders download consumers (the
    /// row badges, the Downloads list), not every `EpisodeState` subscriber.
    pub(crate) downloads: Tracked<DownloadState>,
    /// Connectivity + sync-activity state. Its own signal so the WS latency pongs
    /// (~4×/sec) and Syncing/Saving churn only re-render the navbar status dot +
    /// offline-gated controls, not every `EpisodeState` subscriber.
    pub(crate) connection: Tracked<ConnectionState>,
    /// Session liveness (`auth_expired`). Its own signal so a 401 flips only the
    /// `WorkerProvider` sign-out watcher, not every `EpisodeState` subscriber. The
    /// credentials themselves (`server_url`/`access_token`) live in `ClientConfig`.
    pub(crate) session: Tracked<SessionState>,
    /// Event sink the UI reads. The worker publishes per-domain via
    /// [`SyncService::publish`], which sends only the domains touched since the last
    /// publish (see [`Tracked`]); the provider's applier drains these on the main
    /// thread and writes the matching signals + toast queue.
    pub(crate) events: UnboundedSender<WorkerEvent>,
    /// `Some` once valid auth has been supplied via `SetAuth`; `None` keeps the
    /// worker offline (no placeholder client, no panics).
    pub(crate) api_client: Option<ApiClient>,
    /// Emitter for surfacing runtime errors to the user as toasts (routes through
    /// `events`).
    pub(crate) toasts: WorkerToasts,
    /// Sender side of the worker's internal command channel (set by `run`).
    /// Spawned device-download tasks report completion through it.
    pub(crate) internal_tx: Option<futures::channel::mpsc::UnboundedSender<Command>>,
    /// True while a `do_pull` is mid-flight (it awaits a network round trip, so the cooperative worker can interleave
    /// the next command). At startup `SetAuth`, `WsConnected`, and the queue page's `EnsureDefaultPlaylist` each want a
    /// pull; without this they overlapped into 3-4 redundant `playlists/default` fetches. The guard collapses the burst
    /// to one, later periodic / visibility-driven pulls run normally once it clears.
    pub(crate) pull_in_flight: bool,
    /// Record the last successful pull to coalesce sequential startup triggers that cannot overlap. Explicit refresh,
    /// periodic ticks, reconnect toggles, and subscription-driven pulls bypass the short recency window.
    pub(crate) last_pull_at_ms: Option<u64>,
    /// Podcast ids with an in-flight lazy fetch (from `EnsurePodcast`), so a
    /// burst of rows for the same podcast triggers exactly one request.
    pub(crate) podcasts_in_flight: std::collections::HashSet<i32>,
    /// Episode ids with an in-flight lazy chapter fetch (from
    /// `EnsureEpisodeChapters`), so repeated player-open/track-change triggers for
    /// the same episode collapse to one request.
    pub(crate) episode_chapters_in_flight: std::collections::HashSet<i32>,
    /// Episode ids with an in-flight server-download progress poll, so a repeated
    /// trigger (or row re-render) can't spawn duplicate pollers.
    pub(crate) server_polls_in_flight: std::collections::HashSet<i32>,
    /// Podcast ids the user unsubscribed this session. A page revalidation fetched before the queued `Unsubscribe`
    /// reached the server is STALE and must not re-add them to the pool, `cache_podcasts` filters against this. The
    /// tombstone outlives the outbox op (closing the window where the op drains before the stale page's cache write
    /// lands); cleared on `Subscribe` and on reload.
    pub(crate) unsubscribed: std::collections::HashSet<i32>,
    /// User-toggled "Go Offline" mode (mirrors `ClientConfig.manual_offline`, set
    /// via `SetOffline`). When `true` every network path (pull / drain / history /
    /// `do_pull`) short-circuits and the status stays `Offline`, regardless of the
    /// `api_client` being present.
    pub(crate) manual_offline: bool,
    /// "Add to the front of the queue" preference (mirrors `ClientConfig.playback_prefs.add_to_queue_front`, set via
    /// `SetAddToQueueFront`). When `true`, adding an episode to the *queue* (the default playlist) inserts it at
    /// position 0 instead of appending. Only the queue is affected, adds to any other playlist always append.
    pub(crate) add_to_queue_front: bool,
    /// Device-download chunk size in bytes (mirrors
    /// `ClientConfig.download_prefs.chunk_size`, set via `SetDownloadPrefs`).
    /// `None` = no chunking (fetch the whole file in one request). Read when a
    /// device download starts or resumes.
    pub(crate) device_chunk_bytes: Option<u64>,
    /// Concurrent chunk fetches within a single device download (mirrors
    /// `ClientConfig.download_prefs.parallelism`, set via `SetDownloadPrefs`).
    /// `1` = the historical sequential download. Read alongside `device_chunk_bytes`.
    pub(crate) device_parallelism: u8,
    /// Cancel handle for the running connectivity-WebSocket driver. `Some` while a driver task is live;
    /// dropping/sending stops it (and closes its socket). Set by `start_ws` on `SetAuth`/reconnect, cleared by
    /// `stop_ws` on manual-offline / sign-out / wipe. Re-created on each `SetAuth` so it always uses a fresh token.
    pub(crate) ws_cancel: Option<oneshot::Sender<()>>,
    /// Recent pong round-trip samples (most recent last, capped at [`RTT_WINDOW`]).
    /// Averaged to pick Online vs Degraded; cleared when the connection drops.
    pub(crate) rtt_samples: Vec<u32>,
    /// Budget + backoff bookkeeping for a repeatedly-failing outbox FIFO head
    /// (see [`crate::network::HeadRetry`] and `drain_outbox`).
    pub(crate) head_retry: Option<crate::network::HeadRetry>,
    pub(crate) repaired_rejections: HashSet<u64>,
    pub(crate) last_repair_at_ms: Option<u64>,
    /// Shared mirror of `manual_offline` for DETACHED tasks (device downloads, server-progress polls): they hold their
    /// own `ApiClient` and outlive any one command, so "Go Offline" flips this and the tasks observe it between
    /// polls/chunks and stop transferring, instead of continuing to move bytes after the user turned networking off.
    pub(crate) offline_flag: std::rc::Rc<std::cell::Cell<bool>>,
    /// Scope web drain/download locks to the active account segment. Tabs share IndexedDB, so locks prevent duplicate
    /// operations and interleaved audio chunks; native has one store owner.
    pub(crate) lock_scope: String,
}

impl SyncService {
    /// Create an idle sync service. Networking is gated until `SetAuth` arrives.
    pub fn new(
        store: std::rc::Rc<dyn LocalStore>,
        media: Option<std::rc::Rc<dyn MediaStore>>,
        events: UnboundedSender<WorkerEvent>,
    ) -> Self {
        let toasts = WorkerToasts::new(events.clone());
        Self {
            store,
            deferred_audio_removals: None,
            deferred_server_polls: None,
            deferred_device_starts: None,
            media,
            app_state: Tracked::new(EpisodeState::default()),
            podcasts: Tracked::new(PodcastState::default()),
            playlists: Tracked::new(PlaylistState::default()),
            playbacks: Tracked::new(PlaybackState::default()),
            history: Tracked::new(HistoryState::default()),
            downloads: Tracked::new(DownloadState::default()),
            connection: Tracked::new(ConnectionState::default()),
            session: Tracked::new(SessionState::default()),
            events,
            api_client: None,
            toasts,
            internal_tx: None,
            pull_in_flight: false,
            last_pull_at_ms: None,
            podcasts_in_flight: std::collections::HashSet::new(),
            episode_chapters_in_flight: std::collections::HashSet::new(),
            server_polls_in_flight: std::collections::HashSet::new(),
            unsubscribed: std::collections::HashSet::new(),
            manual_offline: false,
            add_to_queue_front: false,
            // Defaults match `DownloadPrefs::default()` (4 MB chunks, sequential)
            // until the first `SetDownloadPrefs` mirrors the persisted config.
            device_chunk_bytes: Some(4 * 1024 * 1024),
            device_parallelism: 1,
            ws_cancel: None,
            rtt_samples: Vec::new(),
            head_retry: None,
            repaired_rejections: HashSet::new(),
            last_repair_at_ms: None,
            lock_scope: String::new(),
            offline_flag: std::rc::Rc::new(std::cell::Cell::new(false)),
        }
    }

    /// Set the account-segment scope for cross-tab Web Locks (see the
    /// `lock_scope` field). Called by the web runtimes right after
    /// construction, before `run`.
    pub fn set_lock_scope(&mut self, segment: &str) {
        self.lock_scope = segment.to_string();
    }

    /// Publish only dirty domain cells; player state remains independent. Dirty means mutable access, not unequal
    /// values: an equal episode publication may announce changed store contents to list effect A. Do not add a
    /// value-equality gate that suppresses that notification.
    pub(crate) fn publish(&mut self) {
        if self.deferred_audio_removals.is_some() {
            return;
        }
        if self.app_state.take_dirty() {
            let _ = self
                .events
                .unbounded_send(WorkerEvent::Episode((*self.app_state).clone()));
        }
        if self.podcasts.take_dirty() {
            let _ = self
                .events
                .unbounded_send(WorkerEvent::Podcasts((*self.podcasts).clone()));
        }
        if self.playlists.take_dirty() {
            let _ = self
                .events
                .unbounded_send(WorkerEvent::Playlists((*self.playlists).clone()));
        }
        if self.playbacks.take_dirty() {
            let _ = self
                .events
                .unbounded_send(WorkerEvent::Playbacks((*self.playbacks).clone()));
        }
        if self.history.take_dirty() {
            let _ = self
                .events
                .unbounded_send(WorkerEvent::History(*self.history));
        }
        if self.downloads.take_dirty() {
            let _ = self
                .events
                .unbounded_send(WorkerEvent::Downloads((*self.downloads).clone()));
        }
        if self.connection.take_dirty() {
            let _ = self
                .events
                .unbounded_send(WorkerEvent::Connection((*self.connection).clone()));
        }
        if self.session.take_dirty() {
            let _ = self
                .events
                .unbounded_send(WorkerEvent::Session(*self.session));
        }
    }

    /// Main event loop: handle commands and run a periodic pull/drain.
    ///
    /// This is the entry point called from `WorkerProvider` via `use_coroutine`.
    pub async fn run(mut self, commands: futures::channel::mpsc::UnboundedReceiver<Command>) {
        // Merge download reports with UI commands while keeping the worker the sole state writer. Preserve the UI-end
        // None sentinel: the service retains an internal sender, so the merged channel alone never closes and would
        // leave an old-account worker running after unmount.
        let (internal_tx, internal_rx) = futures::channel::mpsc::unbounded::<Command>();
        self.internal_tx = Some(internal_tx);
        let ui_commands = futures::StreamExt::chain(
            futures::StreamExt::map(commands, Some),
            futures::stream::once(std::future::ready(None)),
        );
        let internal = futures::StreamExt::map(internal_rx, Some);
        let mut commands = futures::stream::select(ui_commands, internal);

        // Hydrate initial state from LocalStore (works offline).
        if let Err(e) = self.hydrate_from_store().await {
            error!(error = %e, "Failed to hydrate state from local store");
        }
        self.publish();

        // Reuse one periodic-pull timer across command iterations. Recreating it on each message lets frequent
        // connectivity traffic indefinitely postpone the 60-second pull.
        let mut timer = Box::pin(sleep_secs(PULL_INTERVAL_SECS));
        loop {
            let next_cmd = commands.next();
            match select(Box::pin(next_cmd), timer).await {
                Either::Left((Some(Some(command)), timer_back)) => {
                    // Keep the existing deadline running — a command must not reset it.
                    timer = timer_back;
                    // Capture before `handle_command` consumes the command by value.
                    let reflect = command.reflects_optimistic_state();
                    self.handle_command(command).await;
                    if reflect {
                        // Surface optimistic state before the (maybe slow) drain so
                        // user feedback is immediate even on a throttled link.
                        self.publish();
                    }
                    self.drain_outbox().await;
                    self.publish();
                }
                // `Some(None)` = the UI channel's end sentinel (provider torn
                // down); `None` = every stream ended (unreachable while we hold
                // `internal_tx`, but equally terminal).
                Either::Left((Some(None), _)) | Either::Left((None, _)) => {
                    info!("Command channel closed, shutting down sync worker");
                    break;
                }
                Either::Right(((), _)) => {
                    // Periodic tick: drain any pending ops BEFORE pulling so the
                    // server reflects optimistic queue membership before
                    // `cache_playlists` reads the default playlist back (otherwise
                    // its `episode_ids` overwrite a not-yet-shipped offline add).
                    self.drain_outbox().await;
                    self.do_pull().await;
                    self.publish();
                    // Re-arm the next interval only after the tick actually fired.
                    timer = Box::pin(sleep_secs(PULL_INTERVAL_SECS));
                }
            }
        }
    }

    /// Handle a single command from the UI: apply the optimistic local change,
    /// then enqueue the durable outbox op. The drain step ships it to the server.
    pub(crate) async fn apply_command(&mut self, cmd: Command) {
        debug!(command = ?cmd, "Handling command");
        match cmd {
            Command::Subscribe {
                feed_url,
                title,
                description,
                author,
            } => {
                info!(feed_url = %feed_url, "Subscribing to podcast");
                // Re-subscribing clears the unsubscribe tombstones so server truth
                // (the newly-subscribed podcast, possibly a re-add) flows through.
                self.unsubscribed.clear();
                // No optimistic podcast (no id yet); the drain's follow-up pull
                // surfaces it.
                self.enqueue(OutboxOp::Subscribe {
                    feed_url,
                    title,
                    description,
                    author,
                })
                .await;
            }
            Command::Unsubscribe { podcast_id } => {
                info!(podcast_id, "Unsubscribing from podcast");
                // Tombstone first so a stale in-flight podcast page can't re-add it.
                self.unsubscribed.insert(podcast_id);
                // Drop the cached rows too (not just the in-memory pool) so a reload
                // can't rehydrate the unsubscribed podcast from the store.
                if let Err(e) = self.store.delete_podcast(podcast_id).await {
                    error!(podcast_id, error = %e, "Failed to delete local podcast on unsubscribe");
                }
                self.remove_podcast_locally(podcast_id);
                self.enqueue(OutboxOp::Unsubscribe { podcast_id }).await;
            }
            Command::DeletePlaylist { playlist_id } => {
                info!(playlist_id, "Deleting playlist");
                // Playlist CRUD is ONLINE-ONLY (the create/edit forms disable offline), so this is a DIRECT call, never
                // an outbox op: offline / signed-out toasts, and a failed call surfaces through the shared classify →
                // toast path. Local state is only touched after the server confirms, so a failure leaves the playlist
                // intact.
                let api = (!self.is_offline())
                    .then(|| self.api_client.as_ref().map(|a| a.clone_handle()))
                    .flatten();
                let Some(api) = api else {
                    self.toasts
                        .error("You're offline — can't delete the playlist right now.");
                    return;
                };
                match api.delete_playlist(playlist_id).await {
                    Ok(()) => {
                        self.remove_playlist_locally(playlist_id).await;
                        self.toasts.success("Playlist deleted");
                    }
                    Err(e) => {
                        warn!(playlist_id, error = %e, "Failed to delete playlist");
                        let decision = classify(&e, ToastPolicy::Foreground);
                        self.apply_decision(decision);
                    }
                }
            }
            Command::DownloadToDevice { episode_ids } => {
                let n = episode_ids.len();
                info!(count = n, "Downloading episode(s) to device");
                // One precheck so an offline batch toasts once, not N times (each
                // `start_device_download` would otherwise toast its own error).
                if self.is_offline() {
                    self.toasts
                        .error("You're offline — can't download to this device right now.");
                } else {
                    for episode_id in episode_ids {
                        self.start_device_download(episode_id).await;
                    }
                    // A single row action stays quiet (the spinner is the feedback);
                    // a multiselect batch gets one summary toast.
                    self.bulk_info(n, |n, s| {
                        format!("Downloading {n} episode{s} to this device")
                    });
                }
            }
            Command::DeviceFetchComplete { episode_id, result } => {
                // The spawned task already wrote (or failed to write) the bytes;
                // this is the state transition + user feedback.
                match result {
                    Ok(()) => {
                        info!(episode_id, "Device download complete");
                        self.downloads
                            .client_downloads
                            .insert(episode_id, ClientDownloadState::Downloaded);
                    }
                    Err(msg) => {
                        warn!(episode_id, error = %msg, "Device download failed");
                        self.downloads
                            .client_downloads
                            .insert(episode_id, ClientDownloadState::Failed);
                        self.toasts
                            .error("Couldn't download the episode to this device.");
                    }
                }
                // In-flight progress is done either way.
                self.downloads.download_progress.remove(&episode_id);
            }
            Command::DeviceFetchProgress {
                episode_id,
                percent,
            } => {
                // Only meaningful while still downloading — ignore a late update
                // that raced the completion/removal.
                if matches!(
                    self.downloads.client_downloads.get(&episode_id),
                    Some(ClientDownloadState::Downloading)
                ) {
                    self.downloads.download_progress.insert(episode_id, percent);
                }
            }
            Command::ServerDownloadProgress {
                episode_id,
                percent,
            } => {
                // Only while the server still reports this episode as downloading —
                // drop a late tick that raced the terminal status (mirrors
                // `DeviceFetchProgress`).
                if matches!(
                    self.app_state
                        .episode(episode_id)
                        .map(|e| &e.download_status),
                    Some(DownloadStatus::Downloading)
                ) {
                    self.downloads
                        .server_download_progress
                        .insert(episode_id, percent);
                }
            }
            Command::ServerDownloadComplete {
                episode_id,
                episode,
            } => {
                // Poll ended: drop the dedup guard + the ring, and refresh the
                // durable status. `cache_episodes` also clears any lingering ring
                // for a non-`Downloading` status.
                self.server_polls_in_flight.remove(&episode_id);
                self.downloads.server_download_progress.remove(&episode_id);
                if let Some(ep) = episode {
                    self.cache_episodes(vec![ep]).await;
                } else if matches!(
                    self.app_state
                        .episode(episode_id)
                        .map(|e| &e.download_status),
                    Some(DownloadStatus::Downloading)
                ) {
                    // The poll's final status refresh failed (a transient offline blip at the tail, after its retries).
                    // Don't strand the optimistic `Downloading` badge as a forever-spinner: drop it to a neutral
                    // `NotDownloaded` so the next list/pull revalidation reconciles the true status (rearming a poll
                    // here could busy-loop while offline).
                    self.set_download_status_locally(episode_id, DownloadStatus::NotDownloaded)
                        .await;
                }
            }
            Command::RemoveDownload { episode_ids } => {
                let n = episode_ids.len();
                info!(count = n, "Removing device download(s)");
                for episode_id in episode_ids {
                    self.remove_device_download(episode_id).await;
                }
                self.bulk_success(n, |n, s| {
                    format!("Removed {n} download{s} from this device")
                });
            }
            Command::DownloadOnServer { episode_ids } => {
                let n = episode_ids.len();
                info!(count = n, "Requesting server download(s)");
                // Honest optimism: the server is *about to fetch*, so show
                // `Downloading`; the real status arrives with the next list/get
                // revalidation.
                for &episode_id in &episode_ids {
                    self.set_download_status_locally(episode_id, DownloadStatus::Downloading)
                        .await;
                }
                // Reconcile every requested episode's optimistic `Downloading` badge with a ≈1 Hz progress poll
                // (deduped per id). A batch (n>1) previously started no poll at all, so those badges never got
                // reconciled and could strand; the per-id dedup guard caps it at one poll per episode. (A multiselect
                // batch still summary-toasts below.)
                for &episode_id in &episode_ids {
                    self.ensure_server_progress_poll(episode_id);
                }
                self.enqueue(OutboxOp::TriggerDownload { episode_ids })
                    .await;
                self.bulk_info(n, |n, s| {
                    format!("Downloading {n} episode{s} on the server")
                });
            }
            Command::RemoveServerDownload { episode_ids } => {
                let n = episode_ids.len();
                info!(count = n, "Removing server download(s)");
                // Reset the server-download state locally and drop any device copy
                // too — removing from the server removes it locally as well.
                for &episode_id in &episode_ids {
                    self.set_download_status_locally(episode_id, DownloadStatus::NotDownloaded)
                        .await;
                    self.remove_device_download(episode_id).await;
                }
                self.enqueue(OutboxOp::RemoveServerDownload { episode_ids })
                    .await;
                self.bulk_success(n, |n, s| format!("Removed {n} download{s} from the server"));
            }
            Command::RedownloadDevice { episode_ids } => {
                let n = episode_ids.len();
                info!(count = n, "Device redownload");
                // Don't destroy local copies we can't re-pull: bail before removing
                // when offline (also avoids N offline toasts).
                if self.is_offline() {
                    self.toasts
                        .error("You're offline — can't redownload to this device right now.");
                } else {
                    for episode_id in episode_ids {
                        // Force fresh: remove first so the device flow doesn't dedup.
                        self.remove_device_download(episode_id).await;
                        self.start_device_download(episode_id).await;
                    }
                    self.toasts.info(format!(
                        "Redownloading {n} episode{} on this device",
                        plural(n)
                    ));
                }
            }
            Command::RedownloadOnServer { episode_ids } => {
                let n = episode_ids.len();
                info!(count = n, "Server redownload");
                for &episode_id in &episode_ids {
                    // Optimistic end-state of remove-then-download is `Downloading`.
                    self.set_download_status_locally(episode_id, DownloadStatus::Downloading)
                        .await;
                    // Reconcile that optimistic badge with a progress poll (deduped),
                    // same as `DownloadOnServer` — otherwise a redownload's
                    // `Downloading` badge strands with no poller to resolve it.
                    self.ensure_server_progress_poll(episode_id);
                }
                // Force fresh: remove drains first (inline/200 server-side, resets to
                // NotDownloaded), then the trigger re-fetches. Outbox preserves order.
                self.enqueue(OutboxOp::RemoveServerDownload {
                    episode_ids: episode_ids.clone(),
                })
                .await;
                self.enqueue(OutboxOp::TriggerDownload { episode_ids })
                    .await;
                self.toasts.info(format!(
                    "Redownloading {n} episode{} on the server",
                    plural(n)
                ));
            }
            Command::MarkPlayed { episode_id, played } => {
                info!(episode_id, played, "Marking episode played state");
                self.set_playback_locally(episode_id, |pb| {
                    pb.completed = played;
                    pb.cursor = 0;
                })
                .await;
                // Keep the cached episode facet in lock-step with the overlay so
                // played/unplayed list filters reflect the change immediately.
                let status = if played {
                    PlaybackStatus::Finished
                } else {
                    PlaybackStatus::Unplayed
                };
                self.set_playback_status_locally(episode_id, status).await;
                // MarkPlayed writes the cursor too (0) — queued cursor saves for
                // this episode are superseded the same way (see coalesce_pending_cursor).
                self.coalesce_pending_cursor(episode_id).await;
                self.enqueue(OutboxOp::MarkPlayed { episode_id, played })
                    .await;
            }
            Command::SetCursor { episode_id, cursor } => {
                let cursor = cursor.max(0);
                self.set_playback_locally(episode_id, |pb| pb.cursor = cursor as u64)
                    .await;
                // This save supersedes any still-queued saves for the episode
                // (last-write-wins server-side) — drop them so an offline
                // session can't flood the outbox. See `coalesce_pending_cursor`.
                self.coalesce_pending_cursor(episode_id).await;
                self.enqueue(OutboxOp::SetCursor { episode_id, cursor })
                    .await;
            }
            Command::AddToPlaylist {
                playlist_id,
                episode_ids,
            } => {
                let n = episode_ids.len();
                // Front-of-queue only applies to a SINGLE add to the QUEUE (default
                // playlist); a multiselect batch appends. `Some(0)` = front, `None` =
                // append — the same value drives the optimistic insert and the synced
                // position.
                let position = if n == 1
                    && self.add_to_queue_front
                    && self.playlists.queue_id() == Some(playlist_id)
                {
                    Some(0)
                } else {
                    None
                };
                info!(
                    playlist_id,
                    count = n,
                    ?position,
                    "Adding episode(s) to playlist"
                );
                for &episode_id in &episode_ids {
                    self.add_to_playlist_locally(playlist_id, episode_id, position);
                }
                self.enqueue(OutboxOp::AddToPlaylist {
                    playlist_id,
                    episode_ids,
                    position,
                })
                .await;
                self.persist_playlist(playlist_id).await;
                let dest = self.playlist_dest(playlist_id);
                self.bulk_info(n, |n, s| format!("Added {n} episode{s} to {dest}"));
            }
            Command::RemoveFromPlaylist {
                playlist_id,
                episode_ids,
            } => {
                let n = episode_ids.len();
                info!(playlist_id, count = n, "Removing episode(s) from playlist");
                // Per-playlist cleanup flag: this device also drops its local media
                // copy for episodes it actually removes. Cold cache (playlist not
                // pooled yet) reads as false — the device file just survives.
                let delete_client_file = self
                    .playlists
                    .playlists
                    .iter()
                    .find(|p| p.id == playlist_id)
                    .map(|p| p.on_remove_delete_file_client)
                    .unwrap_or(false);
                let remove: std::collections::HashSet<i32> = episode_ids.iter().copied().collect();
                // Only ids that were actually members — removing a non-member must
                // never nuke a device download.
                let removed: Vec<i32> = self
                    .playlists
                    .episodes_by_playlist
                    .get(&playlist_id)
                    .map(|ids| {
                        ids.iter()
                            .copied()
                            .filter(|id| remove.contains(id))
                            .collect()
                    })
                    .unwrap_or_default();
                if let Some(ids) = self.playlists.episodes_by_playlist.get_mut(&playlist_id) {
                    ids.retain(|id| !remove.contains(id));
                }
                if delete_client_file {
                    for episode_id in removed {
                        // Mirror the server's guard best-effort: another cached
                        // playlist still holding the episode keeps the local copy.
                        let in_other = self
                            .playlists
                            .episodes_by_playlist
                            .iter()
                            .any(|(pid, ids)| *pid != playlist_id && ids.contains(&episode_id));
                        if !in_other {
                            self.remove_device_download(episode_id).await;
                        }
                    }
                }
                self.enqueue(OutboxOp::RemoveFromPlaylist {
                    playlist_id,
                    episode_ids,
                })
                .await;
                self.persist_playlist(playlist_id).await;
                let dest = self.playlist_dest(playlist_id);
                self.bulk_success(n, |n, s| format!("Removed {n} episode{s} from {dest}"));
            }
            Command::MoveInPlaylist {
                playlist_id,
                episode_id,
                to,
            } => {
                info!(playlist_id, episode_id, to, "Moving episode in playlist");
                self.move_in_playlist_locally(playlist_id, episode_id, to);
                self.enqueue(OutboxOp::MoveInPlaylist {
                    playlist_id,
                    episode_id,
                    to,
                })
                .await;
                self.persist_playlist(playlist_id).await;
            }
            Command::MovePlaylist { playlist_id, to } => {
                info!(playlist_id, to, "Moving playlist");
                self.move_playlist_locally(playlist_id, to).await;
                self.enqueue(OutboxOp::MovePlaylist { playlist_id, to })
                    .await;
            }
            Command::ReorderPlaylist {
                playlist_id,
                field,
                direction,
            } => {
                info!(
                    playlist_id,
                    ?field,
                    ?direction,
                    "Reordering playlist episodes"
                );
                self.reorder_playlist_locally(playlist_id, field, direction.clone());
                self.enqueue(OutboxOp::ReorderPlaylist {
                    playlist_id,
                    field,
                    direction,
                })
                .await;
                self.persist_playlist(playlist_id).await;
            }
            Command::CacheEpisodes { episodes } => {
                self.cache_episodes(episodes).await;
            }
            Command::CachePodcasts { podcasts } => {
                self.cache_podcasts(podcasts).await;
            }
            Command::UpdatePlaylist { playlist_id, data } => {
                info!(
                    playlist_id,
                    "Updating playlist offline (optimistic + outbox)"
                );
                self.enqueue(OutboxOp::UpdatePlaylist {
                    playlist_id,
                    data: data.clone(),
                })
                .await;
                self.update_playlist_locally(playlist_id, &data).await;
            }
            Command::CachePlaylists { playlists } => {
                self.cache_playlists(playlists).await;
            }
            Command::UpdatePodcastConfig {
                podcast_id,
                config_id,
                data,
            } => {
                info!(
                    podcast_id,
                    config_id, "Updating podcast config offline (optimistic + outbox)"
                );
                self.update_podcast_config_locally(podcast_id, config_id, &data)
                    .await;
                self.enqueue(OutboxOp::UpdatePodcastConfig { config_id, data })
                    .await;
            }
            Command::RemovePodcastConfig {
                podcast_id,
                config_id,
            } => {
                info!(
                    podcast_id,
                    config_id, "Removing podcast config offline (optimistic + outbox)"
                );
                self.remove_podcast_config_locally(podcast_id).await;
                self.enqueue(OutboxOp::RemovePodcastConfig { podcast_id })
                    .await;
            }
            Command::SetPodcastAutoPlaylists {
                podcast_id,
                playlist_ids,
                add_to_start,
            } => {
                info!(
                    podcast_id,
                    "Setting podcast auto-playlists offline (optimistic + outbox)"
                );
                // Optimistic cache so the form reflects the change immediately;
                // the durable record is the outbox op + the server.
                self.podcasts
                    .auto_playlists_by_podcast
                    .insert(podcast_id, playlist_ids.clone());
                self.podcasts
                    .auto_playlist_add_to_start_by_podcast
                    .insert(podcast_id, add_to_start);
                let rows = self.podcasts.auto_playlists_by_podcast[&podcast_id]
                    .iter()
                    .map(|id| halogen_wire::PodcastAutoPlaylistData {
                        podcast_id,
                        playlist_id: *id,
                        add_to_start,
                    })
                    .collect::<Vec<_>>();
                if let Err(error) = self.store.replace_auto_playlists(podcast_id, &rows).await {
                    error!(%error, "Failed to cache auto-playlist selection");
                }
                self.enqueue(OutboxOp::SetPodcastAutoPlaylists {
                    podcast_id,
                    playlist_ids,
                    add_to_start,
                })
                .await;
            }
            Command::CacheAutoPlaylists {
                podcast_id,
                playlist_ids,
                add_to_start,
            } => {
                // Pure cache write after a direct (online) fetch or save.
                self.podcasts
                    .auto_playlists_by_podcast
                    .insert(podcast_id, playlist_ids);
                self.podcasts
                    .auto_playlist_add_to_start_by_podcast
                    .insert(podcast_id, add_to_start);
                let rows = self.podcasts.auto_playlists_by_podcast[&podcast_id]
                    .iter()
                    .map(|id| halogen_wire::PodcastAutoPlaylistData {
                        podcast_id,
                        playlist_id: *id,
                        add_to_start,
                    })
                    .collect::<Vec<_>>();
                if let Err(error) = self.store.replace_auto_playlists(podcast_id, &rows).await {
                    error!(%error, "Failed to cache auto-playlist selection");
                }
            }
            Command::EnsurePodcast { podcast_id } => {
                self.ensure_podcast(podcast_id);
            }
            Command::PodcastFetched {
                podcast_id,
                podcast,
            } => {
                self.podcasts_in_flight.remove(&podcast_id);
                if let Some(p) = podcast {
                    self.cache_podcasts(vec![p]).await;
                }
            }
            Command::PodcastsFetched {
                podcast_ids,
                podcasts,
            } => {
                // Clear every requested guard (even ids the server omitted, so they
                // can be retried), then merge whatever came back into the pool.
                for id in &podcast_ids {
                    self.podcasts_in_flight.remove(id);
                }
                if let Some(ps) = podcasts {
                    self.cache_podcasts(ps).await;
                }
            }
            Command::EnsureEpisodeChapters { episode_id } => {
                self.ensure_episode_chapters(episode_id);
            }
            Command::EpisodeChaptersFetched {
                episode_id,
                episode,
            } => {
                self.episode_chapters_in_flight.remove(&episode_id);
                if let Some(ep) = episode {
                    // `cache_episodes` carries the chapters in; the sticky merge
                    // there keeps them through later chapter-less list refreshes.
                    self.cache_episodes(vec![ep]).await;
                }
            }
            Command::EnsureDefaultPlaylist => {
                self.ensure_default_playlist().await;
            }
            Command::LoadHistory { reset } => {
                self.load_history(reset).await;
            }
            Command::SetOffline(offline) => {
                self.manual_offline = offline;
                // Detached tasks (downloads, progress polls) watch this mirror.
                self.offline_flag.set(offline);
                if offline {
                    info!("Manual offline enabled — skipping all network sync");
                    // Tear the connectivity socket down: manual offline means no
                    // network at all, and a live socket would keep flipping us back.
                    self.stop_ws();
                    self.set_offline_status();
                    self.publish();
                } else {
                    info!("Manual offline disabled — reconnecting");
                    // Rebuild the connectivity socket, then flush queued ops BEFORE pulling so optimistic queue
                    // membership reaches the server before `cache_playlists` reads the default playlist back (its
                    // `episode_ids` would otherwise overwrite a not-yet-shipped offline add).
                    self.start_ws();
                    self.drain_outbox().await;
                    self.do_pull().await;
                    self.publish();
                }
            }
            Command::SetAddToQueueFront(front) => {
                // No network/state effect — just remember the preference for the next
                // queue add (see the `AddToPlaylist` handler).
                self.add_to_queue_front = front;
            }
            Command::SetDownloadPrefs {
                chunk_bytes,
                parallelism,
            } => {
                // No network/state effect — remembered for the next device download
                // start/resume (see `start_device_download` / `download_audio`).
                self.device_chunk_bytes = chunk_bytes;
                self.device_parallelism = parallelism.max(1);
            }
            Command::RefreshNow => {
                info!("Manual refresh requested");
                // Drain BEFORE pulling so a not-yet-shipped optimistic queue add
                // reaches the server before `cache_playlists` reads the default
                // playlist back (its `episode_ids` would otherwise overwrite it).
                // The post-handler loop drain is then a no-op on the emptied outbox.
                self.drain_outbox().await;
                self.do_pull().await;
            }
            Command::SetAuth { server_url, token } => {
                info!("Auth credentials updated");
                self.set_auth(server_url, token.0).await;
            }
            Command::RemoveLocalEpisodeData { episode_id } => {
                info!(episode_id, "Removing local data for episode");
                self.remove_local_episode_data(episode_id).await;
            }
            Command::RemoveLocalPodcastData { podcast_id } => {
                info!(podcast_id, "Removing local data for podcast");
                self.remove_local_podcast_data(podcast_id).await;
            }
            Command::WsConnected => {
                // Ignore a late event that raced a "Go Offline" toggle (the driver
                // is being torn down; manual offline must stay offline).
                if self.manual_offline {
                    return;
                }
                info!("WebSocket connected");
                self.set_online_status();
                self.publish();
                // Drain before reconnect pulling so server playlist rows include optimistic membership edits. Coalesce
                // a cold-start socket event with the recent auth pull; a later reconnect still pulls immediately.
                self.drain_outbox().await;
                self.coalesced_pull().await;
            }
            Command::WsDisconnected => {
                debug!("WebSocket disconnected");
                self.set_offline_status();
                self.publish();
            }
            Command::WsLatency { rtt_ms } => {
                if self.manual_offline {
                    return;
                }
                self.record_latency(rtt_ms);
                self.publish();
            }
            Command::WsAuthExpired => {
                warn!("WebSocket ticket unauthorized — flagging session expired");
                self.session.auth_expired = true;
                self.publish();
            }
            Command::WipeLocal => {
                info!("Wiping local data");
                self.stop_ws();
                if let Err(e) = self.store.clear().await {
                    error!(error = %e, "Failed to clear local store");
                }
                if let Some(media) = self.media.as_ref()
                    && let Err(e) = media.clear().await
                {
                    error!(error = %e, "Failed to clear device audio");
                }
                // Device logs: drop the in-memory ring + the not-yet-flushed
                // queue, then wipe the persisted log store (native file / wasm
                // IndexedDB `halogen.logs`).
                halogen_webui_logging::clear();
                let _ = halogen_webui_logging::drain_pending();
                halogen_webui_logging::store::clear().await;
                *self.app_state = EpisodeState::default();
                *self.podcasts = PodcastState::default();
                *self.playlists = PlaylistState::default();
                *self.playbacks = PlaybackState::default();
                *self.history = HistoryState::default();
                *self.downloads = DownloadState::default();
                *self.connection = ConnectionState::default();
                *self.session = SessionState::default();
                self.api_client = None;
                self.rtt_samples.clear();
                // Clear the in-flight dedup guards: a detached fetch/poll that lands
                // after the wipe must not be treated as still-pending, and a stale id
                // left here would make every later `ensure_*` a permanent no-op
                // (`spawn_deduped` returns early on an already-present key).
                self.podcasts_in_flight.clear();
                self.episode_chapters_in_flight.clear();
                self.server_polls_in_flight.clear();
                self.publish();
            }
            Command::SignOut => {
                info!("Signing out (dropping auth, keeping local cache)");
                self.stop_ws();
                self.api_client = None;
                self.session.auth_expired = false;
                // Same guard-clear as WipeLocal: drop stale in-flight ids so post
                // sign-out `ensure_*` calls aren't wedged into a permanent no-op.
                self.podcasts_in_flight.clear();
                self.episode_chapters_in_flight.clear();
                self.server_polls_in_flight.clear();
                self.set_offline_status();
                self.publish();
            }
        }
    }

    /// Hydrate EpisodeState from LocalStore on launch.
    async fn hydrate_from_store(&mut self) -> Result<()> {
        // Podcasts: load the cached pool for O(1) detail/title lookups + an offline
        // starting point for the (now paged) list, which revalidates from the server
        // page-by-page. This is a cheap LOCAL read — the eager *server* pull is gone.
        let podcasts = self.store.list_podcasts().await?;
        let mut by_id: HashMap<i32, EpisodeData> = HashMap::new();
        let mut by_podcast: HashMap<i32, Vec<i32>> = HashMap::new();
        for p in &podcasts {
            for ep in self.store.list_episodes(p.id).await? {
                by_podcast.entry(ep.podcast_id).or_default().push(ep.id);
                by_id.insert(ep.id, ep);
            }
        }
        self.podcasts.podcasts_by_id = podcasts.into_iter().map(|p| (p.id, p)).collect();
        self.app_state.episodes_by_id = by_id;
        self.app_state.episodes_by_podcast = by_podcast;
        for (id, rows) in self.store.list_auto_playlists().await? {
            self.podcasts
                .auto_playlist_add_to_start_by_podcast
                .insert(id, rows.first().and_then(|row| row.add_to_start));
            self.podcasts
                .auto_playlists_by_podcast
                .insert(id, rows.into_iter().map(|row| row.playlist_id).collect());
        }
        let playbacks = self.store.list_playbacks().await?;
        self.playbacks.playbacks = playbacks.into_iter().map(|p| (p.episode_id, p)).collect();
        // Device downloads: committed bytes on disk are the ground truth. Ids
        // present in the media store hydrate as `Downloaded`. In-flight downloads
        // aren't restored here (the api client isn't up yet) — `set_auth` calls
        // `resume_partial_downloads` to pick up any staged partials once authed.
        if let Some(media) = self.media.as_ref() {
            match media.list_ids().await {
                Ok(ids) => {
                    self.downloads.client_downloads = ids
                        .into_iter()
                        .map(|id| (id, ClientDownloadState::Downloaded))
                        .collect();
                }
                Err(e) => error!(error = %e, "Failed to list device downloads"),
            }
        }
        // Playlists carry their ordered episode ids; the membership index is
        // rebuilt from them (episode bodies resolve from the pool on demand).
        let playlists = self.store.list_playlists().await?;
        let mut by_playlist: HashMap<i32, Vec<i32>> = HashMap::new();
        for pl in &playlists {
            if let Some(ids) = &pl.episode_ids {
                by_playlist.insert(pl.id, ids.clone());
            }
        }
        self.playlists.playlists = playlists;
        self.playlists.episodes_by_playlist = by_playlist;
        // Resolve the queue from the locally-cached default so queue-dependent UI (the "Add to queue" action, /queue)
        // works offline on a cold start. Only the server can assert there's genuinely no queue, so don't mark Absent
        // here, a missing local default stays Unknown until the online `ensure_default_playlist` confirms it.
        self.playlists.recompute_queue(false);
        Ok(())
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use std::rc::Rc;

    use futures::executor::block_on;
    use halogen_webui_store::NativeLocalStore;
    use halogen_wire::PlaylistData;

    use super::*;
    use crate::SyncService;

    /// A service over a fresh throwaway SQLite store (no media, no api client) —
    /// mirrors `data_ops::cache_playlists_tests::test_service`. Media is `None`,
    /// so `remove_device_download` only clears the client state maps.
    fn test_service(db: &str) -> SyncService {
        let path = std::env::temp_dir().join(format!(
            "halogen-worker-test-{}-{db}.db",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let store = Rc::new(NativeLocalStore::open(path).expect("open test store"));
        let (tx, _rx) = futures::channel::mpsc::unbounded();
        SyncService::new(store, None, tx)
    }

    /// Repeated cursor saves for one episode must coalesce to a single pending
    /// outbox op (last value wins); other episodes' saves are untouched. Pre-fix
    /// a long offline session queued one op per ~10s save and reconnect drained
    /// them one serial round-trip at a time.
    #[test]
    fn cursor_saves_coalesce_in_outbox() {
        block_on(async {
            let mut svc = test_service("cursor-coalesce");
            for cursor in [10, 20, 30] {
                svc.handle_command(Command::SetCursor {
                    episode_id: 1,
                    cursor,
                })
                .await;
            }
            svc.handle_command(Command::SetCursor {
                episode_id: 2,
                cursor: 99,
            })
            .await;
            let cursors: Vec<(i32, i64)> = svc
                .store
                .pending()
                .await
                .expect("pending")
                .into_iter()
                .filter_map(|(_, op)| match op {
                    OutboxOp::SetCursor { episode_id, cursor } => Some((episode_id, cursor)),
                    _ => None,
                })
                .collect();
            assert_eq!(cursors, vec![(1, 30), (2, 99)]);
        });
    }

    /// Cold start is Unknown (not offline — no boot-time offline flashes); the
    /// first pull resolves it, here to Offline (no api client).
    #[test]
    fn boot_status_unknown_resolves_offline_on_authless_pull() {
        block_on(async {
            let mut svc = test_service("status-unknown");
            assert!(svc.connection.sync_status.is_unknown());
            assert!(!svc.connection.sync_status.is_offline());
            svc.do_pull().await;
            assert!(svc.connection.sync_status.is_offline());
        });
    }

    /// `run` must END when the UI command channel closes, even though the service holds its own `internal_tx` into the
    /// merged stream. Pre-fix the merged stream went pending and the 60s timer kept the loop alive forever, on the web
    /// fallback that was a zombie second writer surviving account switch.
    #[tokio::test]
    async fn run_exits_when_ui_channel_closes() {
        let svc = test_service("run-exit");
        let (tx, rx) = futures::channel::mpsc::unbounded::<Command>();
        drop(tx);
        tokio::time::timeout(std::time::Duration::from_secs(10), svc.run(rx))
            .await
            .expect("run() must exit when the UI command channel closes");
    }

    fn playlist(id: i32, delete_client: bool) -> PlaylistData {
        let now = chrono::Utc::now();
        PlaylistData {
            id,
            name: format!("Playlist {id}"),
            description: None,
            is_default: false,
            position: 0,
            on_remove_delete_file_server: false,
            on_remove_delete_file_client: delete_client,
            created_at: now,
            updated_at: now,
            episode_ids: None,
            episode_playlist: None,
        }
    }

    /// `on_remove_delete_file_client` set: removing an episode also drops its
    /// device download — but only for ids that were actually members, and other
    /// episodes' downloads are untouched.
    #[test]
    fn remove_from_flagged_playlist_drops_device_download() {
        let mut svc = test_service("rm-client-flag");
        block_on(async {
            svc.playlists.playlists.push(playlist(1, true));
            svc.playlists.episodes_by_playlist.insert(1, vec![7, 9]);
            for id in [7, 8, 9] {
                svc.downloads
                    .client_downloads
                    .insert(id, ClientDownloadState::Downloaded);
            }

            // 8 is NOT a member — removing it must never nuke its download.
            svc.handle_command(Command::RemoveFromPlaylist {
                playlist_id: 1,
                episode_ids: vec![7, 8],
            })
            .await;

            assert_eq!(svc.playlists.episodes_by_playlist.get(&1), Some(&vec![9]));
            assert!(
                !svc.downloads.client_downloads.contains_key(&7),
                "removed member's device download must be dropped"
            );
            assert!(
                svc.downloads.client_downloads.contains_key(&8),
                "non-member id must keep its device download"
            );
            assert!(
                svc.downloads.client_downloads.contains_key(&9),
                "episode still in the playlist keeps its download"
            );
        });
    }

    /// Flag unset (default): removal never touches device downloads.
    #[test]
    fn remove_from_unflagged_playlist_keeps_device_download() {
        let mut svc = test_service("rm-no-flag");
        block_on(async {
            svc.playlists.playlists.push(playlist(1, false));
            svc.playlists.episodes_by_playlist.insert(1, vec![7]);
            svc.downloads
                .client_downloads
                .insert(7, ClientDownloadState::Downloaded);

            svc.handle_command(Command::RemoveFromPlaylist {
                playlist_id: 1,
                episode_ids: vec![7],
            })
            .await;

            assert_eq!(
                svc.playlists.episodes_by_playlist.get(&1),
                Some(&Vec::new())
            );
            assert!(
                svc.downloads.client_downloads.contains_key(&7),
                "flag off — the device download must survive"
            );
        });
    }

    /// Flag set but the episode is still in ANOTHER cached playlist: the device
    /// copy survives (mirrors the server's shared-membership guard).
    #[test]
    fn remove_from_flagged_playlist_spares_episode_in_other_playlist() {
        let mut svc = test_service("rm-flag-shared");
        block_on(async {
            svc.playlists.playlists.push(playlist(1, true));
            svc.playlists.playlists.push(playlist(2, false));
            svc.playlists.episodes_by_playlist.insert(1, vec![7]);
            svc.playlists.episodes_by_playlist.insert(2, vec![7]);
            svc.downloads
                .client_downloads
                .insert(7, ClientDownloadState::Downloaded);

            svc.handle_command(Command::RemoveFromPlaylist {
                playlist_id: 1,
                episode_ids: vec![7],
            })
            .await;

            assert_eq!(
                svc.playlists.episodes_by_playlist.get(&1),
                Some(&Vec::new())
            );
            assert!(
                svc.downloads.client_downloads.contains_key(&7),
                "episode still in another playlist — device download must survive"
            );
        });
    }
}
