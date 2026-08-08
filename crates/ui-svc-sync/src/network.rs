//! Networking + sync orchestration for [`SyncService`] — split from the command
//! router (`worker`) and the optimistic local mutators (`data_ops`). These are the
//! methods that touch the network (`api_client`) and reconcile the result into
//! `EpisodeState`: the device-download kickoff, the offline-probe pull, History
//! `/playbacks` paging, the outbox drain, and `SetAuth`.

use chrono::Utc;
use halogen_api::ApiClient;
use halogen_wire::{
    DownloadStatus, Order, OrderDirection, Pagination, PlaybackData, PlaybackListParams,
};
use std::collections::HashSet;
use url::Url;

use super::{PULL_PAGE_SIZE, SyncService, device_download_task, sleep_secs, spawn_task};
use halogen_ui_appstate::state::ClientDownloadState;
use halogen_ui_logging::{debug, error, warn};
use halogen_ui_svc_store::outbox::OutboxOp;
use halogen_ui_toast::{
    ToastDecision, ToastLevel, ToastPolicy, classify, is_countable_failure, is_permanent_failure,
};

/// Consecutive countable failures (see [`halogen_ui_toast::is_countable_failure`])
/// of the same outbox head op before it is dead-lettered. Combined with the
/// [`drain_skips`] backoff this is roughly half an hour of *persistent*
/// deterministic failure at the ~15s command-drain cadence — long enough to ride
/// out a server outage window (a real outage is usually `Transport`, which never
/// counts), short enough that one poisoned op can't wedge the queue for days.
pub(crate) const OUTBOX_DEAD_LETTER_ATTEMPTS: u32 = 10;

/// Drains to skip before re-attempting a head op with `failures` consecutive
/// countable failures: 1, 3, 7, then 15 (cap). At the ~15s WS-latency drain
/// cadence the cap is ~4 min between attempts — without this, the drain-after-
/// every-command loop re-sent (and re-toasted) a failing op several times a
/// minute indefinitely.
pub(crate) fn drain_skips(failures: u32) -> u32 {
    (1u32 << failures.min(4)) - 1
}

/// Retry bookkeeping for a failing outbox FIFO head (see
/// [`SyncService::drain_outbox`]). In-memory only — a worker restart grants a
/// fresh budget, which is fine: a deterministic failure re-exhausts it quickly,
/// and anything transient deserves the fresh start.
pub(crate) struct HeadRetry {
    pub(crate) op_id: u64,
    pub(crate) failures: u32,
    pub(crate) skip_drains: u32,
}

impl SyncService {
    /// Begin a device download: mark `Downloading`, ensure the server fetches
    /// its copy (durable outbox op, idempotent server-side), and spawn the
    /// detached byte-fetch task that reports back via `DeviceFetchComplete`.
    pub(super) async fn start_device_download(&mut self, episode_id: i32) {
        if self.manual_offline {
            // No server: can't fetch bytes. Leave state untouched, just say why.
            self.toasts
                .info("You're offline — reconnect to download episodes.");
            return;
        }
        // Idempotence: the badge and a play-click can both request the same
        // episode; one in-flight task is enough, and re-downloading a stored
        // episode is pointless (`RemoveDownload` first to force it).
        match self.downloads.client_downloads.get(&episode_id) {
            Some(ClientDownloadState::Downloading) | Some(ClientDownloadState::Downloaded) => {
                return;
            }
            Some(ClientDownloadState::Failed) | None => {}
        }

        let (Some(tx), Some(media)) = (self.internal_tx.clone(), self.media.clone()) else {
            // No media backend (or called before `run`): fail fast and honestly.
            self.downloads
                .client_downloads
                .insert(episode_id, ClientDownloadState::Failed);
            self.toasts
                .error("Device storage is unavailable — can't download.");
            return;
        };
        // The task needs its own client (cloned base + token snapshot) — taken
        // up front so the `api_client` borrow ends before `enqueue(&mut self)`.
        let Some(api) = self.api_client.as_ref().map(|a| a.clone_handle()) else {
            self.downloads
                .client_downloads
                .insert(episode_id, ClientDownloadState::Failed);
            self.toasts.error("Sign in to download episodes.");
            return;
        };

        // Fail fast when we already know we're offline. A device download pulls
        // the server's copy (never the RSS origin), so with the server
        // unreachable there's nothing to do — surface the error now instead of
        // polling a dead server for the full `DEVICE_POLL_TRIES` budget (~2 min).
        // The user retries once connectivity returns (sync_status → Online).
        // `Unknown` (cold start) deliberately proceeds — optimistic local-first;
        // a transport failure resolves it to Offline.
        if matches!(
            self.connection.sync_status,
            halogen_ui_appstate::state::SyncStatus::Offline
        ) {
            self.downloads
                .client_downloads
                .insert(episode_id, ClientDownloadState::Failed);
            self.toasts
                .error("You're offline — can't download to this device right now.");
            return;
        }

        self.downloads
            .client_downloads
            .insert(episode_id, ClientDownloadState::Downloading);
        // Ask the server to fetch its copy first (the device pulls from the
        // server, never the RSS origin). Durable: survives offline/restart.
        self.enqueue(OutboxOp::TriggerDownload {
            episode_ids: vec![episode_id],
        })
        .await;
        let server_ready = self
            .app_state
            .episode(episode_id)
            .map(|e| e.download_status.clone())
            == Some(DownloadStatus::Downloaded);
        // Surface the SERVER fetch as its own badge phase (the cloud ring) when the
        // server doesn't already hold the file: mark it `Downloading` locally and
        // poll the server's progress tracker, so the badge fills as the *server*
        // pulls the file — then the device byte-pull below takes over the ring.
        // When the server already has it, the device pull starts right away and
        // there's no cloud phase to show.
        if !server_ready {
            self.set_download_status_locally(episode_id, DownloadStatus::Downloading)
                .await;
            self.ensure_server_progress_poll(episode_id);
        }
        if !spawn_task(device_download_task(
            episode_id,
            self.download_lock_name(episode_id),
            self.offline_flag.clone(),
            api,
            media,
            tx,
            server_ready,
            self.device_chunk_bytes,
            self.device_parallelism,
        )) {
            error!(
                episode_id,
                "device download unsupported on this runtime (no dioxus runtime or LocalSet)"
            );
            self.downloads
                .client_downloads
                .insert(episode_id, ClientDownloadState::Failed);
            self.toasts
                .error("Device downloads aren't supported on this platform yet.");
        }
    }

    /// How long to hold off a boot-time download resume so the multi-megabyte
    /// audio byte-fetches don't contend with the initial page render (critical
    /// data + list/cover art) for bandwidth and the worker's CPU. The worker can't
    /// observe paint, so this is a fixed grace window; user-initiated downloads
    /// (`start_device_download`) are NOT delayed — only this background resume.
    const RESUME_DELAY_SECS: u64 = 5;

    /// Re-enter device downloads interrupted by a page refresh / app restart.
    ///
    /// Each staged partial (its bytes durable in the media store) resumes from its
    /// byte offset via the same `device_download_task` the kickoff uses —
    /// `download_audio` reads the offset from `MediaStore::partial`. Run
    /// after `set_auth` brings the api client up (a download pulls the SERVER's
    /// copy, so it needs auth + reachability); skips ids already committed or
    /// in-flight, so it's safe to call more than once.
    pub(super) async fn resume_partial_downloads(&mut self) {
        // Offline: nothing to pull from. The partials wait on disk for the next
        // authenticated start (a fresh `SetAuth`) or a manual retry. `Unknown`
        // (cold start) deliberately proceeds — a transport failure resolves it.
        if self.manual_offline
            || matches!(
                self.connection.sync_status,
                halogen_ui_appstate::state::SyncStatus::Offline
            )
        {
            return;
        }
        let (Some(media), Some(tx)) = (self.media.clone(), self.internal_tx.clone()) else {
            return;
        };
        let Some(api) = self.api_client.as_ref().map(|a| a.clone_handle()) else {
            return;
        };
        let partials = match media.list_partials().await {
            Ok(p) => p,
            Err(e) => {
                error!(error = %e, "failed to list partial downloads to resume");
                return;
            }
        };
        for id in partials {
            // Committed wins (a commit-crash can leave a stale partial); skip an
            // already in-flight download too.
            match self.downloads.client_downloads.get(&id) {
                Some(ClientDownloadState::Downloaded) | Some(ClientDownloadState::Downloading) => {
                    continue;
                }
                _ => {}
            }
            self.downloads
                .client_downloads
                .insert(id, ClientDownloadState::Downloading);
            // Seed the progress bar from the staged offset so the badge shows real
            // percent immediately (absent until the first byte otherwise).
            if let Ok(Some(info)) = media.partial(id).await
                && let Some(total) = info.total.filter(|t| *t > 0)
            {
                let pct = ((info.downloaded * 100) / total).min(100) as u8;
                self.downloads.download_progress.insert(id, pct);
            }
            // The server copy already exists (it's how this partial got bytes), but
            // derive readiness from the cached status; the task re-polls if unsure.
            let server_ready = self
                .app_state
                .episode(id)
                .map(|e| e.download_status.clone())
                == Some(DownloadStatus::Downloaded);
            let task = device_download_task(
                id,
                self.download_lock_name(id),
                self.offline_flag.clone(),
                api.clone_handle(),
                media.clone(),
                tx.clone(),
                server_ready,
                self.device_chunk_bytes,
                self.device_parallelism,
            );
            // Hold the byte-fetch off for the grace window so it doesn't fight the
            // initial render; the task future is inert until awaited.
            let delay = Self::RESUME_DELAY_SECS;
            if !spawn_task(async move {
                sleep_secs(delay).await;
                task.await;
            }) {
                error!(id, "could not spawn device-download resume task");
                self.downloads
                    .client_downloads
                    .insert(id, ClientDownloadState::Failed);
                // Drop the seeded percent so a Failed badge doesn't show a stale
                // progress figure (mirrors `DeviceFetchComplete`).
                self.downloads.download_progress.remove(&id);
            }
        }
    }

    /// Pull latest data from the server and reconcile with local state.
    pub(super) async fn do_pull(&mut self) {
        if self.pull_in_flight {
            // A pull is already mid-flight (it awaits a network round trip, so the
            // cooperative worker interleaves the next command). The overlapping
            // startup triggers — SetAuth, WsConnected, and the queue page's
            // EnsureDefaultPlaylist — would otherwise each re-fetch the same default
            // playlist; let the one already running refresh everything for all of
            // them. (Periodic / visibility-driven pulls run normally once it clears.)
            return;
        }
        if self.manual_offline {
            // "Go Offline" is on — never touch the network, even on an explicit
            // RefreshNow / SetAuth. Hold at Offline.
            self.set_offline_status();
            self.publish();
            return;
        }
        if self.api_client.is_none() {
            // No auth yet — stay offline until SetAuth arrives.
            self.set_offline_status();
            self.publish();
            return;
        }
        self.connection.sync_status = halogen_ui_appstate::state::SyncStatus::Syncing {
            phase: halogen_ui_appstate::state::SyncPhase::Pull,
        };
        // Claim the in-flight guard for the whole pull; cleared on every exit below.
        self.pull_in_flight = true;

        // Neither podcasts nor episodes are bulk-pulled. Both are
        // offline-first/paged: the podcasts list and every episode list read their
        // pages from the store, revalidate page-by-page from the server, and hand
        // the results back via `CachePodcasts`/`CacheEpisodes`. (Bulk all-podcasts
        // + all-episodes pulls would churn the server and overflow `localStorage`.)

        // Playlists are no longer bulk-pulled either — the playlists list, the
        // "add to playlist" picker, and the auto-playlists form page the server and
        // augment the pool on demand (like podcasts/episodes). The DEFAULT playlist
        // (the queue) IS fetched here: it's the offline probe (first network call, a
        // failure means offline) AND the authoritative queue resolver. It augments
        // the pool by id — it must NOT wipe other lazily-cached playlists.
        // Drop the `api` borrow before touching `&mut self` helpers (the error
        // branch already does this). Holding the result lets us route the default
        // playlist through the canonical `cache_playlists` path below — which clears
        // stale `is_default` rows and calls `recompute_queue` — instead of an inline
        // merge that diverged from it.
        // Scope the `api` borrow so it ends before the `&mut self` reconciliation
        // below (cache_playlists / status helpers) — and after setting the guard.
        let result = {
            let api = self
                .api_client
                .as_ref()
                .expect("api_client present (checked above)");
            api.get_default_playlist().await
        };
        match result {
            Ok(Some(def)) => {
                // Funnel through `cache_playlists` so the default playlist gets the
                // same treatment as every other cached playlist: persistence, the
                // membership-preservation invariant (a row without `episode_ids`
                // must NOT wipe optimistic queue membership an outbox op hasn't
                // drained yet), clearing `is_default` on any other cached playlist,
                // and a canonical `recompute_queue` (rather than a raw
                // `queue = Present(id)`). `def.is_default` is true for the default
                // playlist, so `recompute_queue` resolves the queue to it.
                self.cache_playlists(vec![def]).await;
            }
            Ok(None) => self.playlists.queue = halogen_ui_appstate::QueueState::Absent,
            Err(e) => {
                // The queue fetch corroborates the WS offline probe. Funnel through
                // the shared `apply_decision` (toast + auth-expired) like the outbox
                // drain, then reflect *transport* failures as offline (a 5xx/
                // validation error is a reachable server, so it isn't offline).
                let decision = classify(&e, ToastPolicy::Background);
                let went_offline = matches!(decision, ToastDecision::Offline);
                self.apply_decision(decision);
                if went_offline {
                    self.set_offline_status();
                } else {
                    // A reachable server that 5xx'd / failed validation isn't
                    // offline — resolve the status instead of leaving it stuck on
                    // `Syncing{Pull}` (drain_outbox no-ops on an empty outbox and
                    // won't fix it). Mirrors the success path's `set_online_status`.
                    self.set_online_status();
                }
                self.connection.last_error = Some(format!("Sync failed: {e}"));
                warn!(error = %e, "Pull failed");
                self.pull_in_flight = false;
                return;
            }
        }

        // Playbacks are NO LONGER bulk-pulled here. The resume cursor rides each
        // episode page (server `EpisodeInclude::Playback`, overlay-wins on the
        // client), and History pages `GET /playbacks` on demand (`load_history`).
        // This was the last full-set network pull in the periodic loop.

        self.set_online_status();
        self.connection.last_error = None;
        self.connection.last_synced_at = Some(Utc::now());
        // Stamp the recency clock only on a pull that reached the server (this
        // success path + the `Ok(None)` empty-queue path, both of which fall through
        // here). The transient-error branch returns early WITHOUT stamping, so an
        // offline blip never suppresses the next startup pull. See `coalesced_pull`.
        self.last_pull_at_ms = Some(halogen_ui_platform::time::now_ms());
        self.pull_in_flight = false;
        self.publish();
    }

    /// Milliseconds within which a just-completed pull's data is considered fresh
    /// enough to skip a redundant startup pull. Long enough to swallow the boot
    /// SetAuth → WsConnected → EnsureDefaultPlaylist burst (each a sub-second round
    /// trip apart), far below the 60s periodic interval.
    const PULL_COALESCE_MS: u64 = 2_000;

    /// Whether a server-reaching pull completed within [`Self::PULL_COALESCE_MS`].
    pub(super) fn recently_pulled(&self) -> bool {
        is_recent(
            self.last_pull_at_ms,
            halogen_ui_platform::time::now_ms(),
            Self::PULL_COALESCE_MS,
        )
    }

    /// A startup/reconnect pull that collapses with a just-completed one.
    ///
    /// `do_pull`'s `pull_in_flight` guard only catches pulls that *overlap in time*;
    /// at a fast cold boot the SetAuth / WsConnected / EnsureDefaultPlaylist triggers
    /// fire one-after-another (each after the prior's network round trip), so the
    /// guard is already clear and each re-fetches `/playlists/default`. This skips the
    /// pull when a successful one landed moments ago — collapsing the boot triple to
    /// one. Explicit refresh, the periodic tick, the reconnect toggle, and
    /// subscription-driven pulls call `do_pull` directly and always run.
    pub(super) async fn coalesced_pull(&mut self) {
        if self.recently_pulled() {
            debug!("coalesced_pull: skipping (pulled within the recency window)");
            return;
        }
        self.do_pull().await;
    }

    /// Page the History source: fetch one `GET /playbacks` page (updated_at desc)
    /// and merge it into the `playbacks` overlay + local store. The History list
    /// derives its ordered membership by sorting that overlay, so growing it here
    /// (and on boot via `hydrate_from_store`) is what makes History offline-first
    /// and server-paged — no bulk pull. `reset` restarts from page 0 (mount /
    /// pull-to-refresh); otherwise advances `history_next_page`. No-op once the
    /// server reports no more pages (unless resetting).
    pub(super) async fn load_history(&mut self, reset: bool) {
        if self.manual_offline {
            // Offline: History renders from the hydrated overlay only.
            return;
        }
        let Some(api) = self.api_client.as_ref() else {
            return;
        };
        if reset {
            self.history.history_next_page = 0;
            self.history.history_has_more = true;
        } else if !self.history.history_has_more {
            return;
        }
        let page = self.history.history_next_page;
        let params = PlaybackListParams {
            pagination: Some(Pagination {
                page,
                size: PULL_PAGE_SIZE,
            }),
            order: Some(Order {
                direction: OrderDirection::Desc,
                order_by: "updated_at".to_string(),
            }),
            episode_id: None,
        };
        match api.list_playbacks(params).await {
            Ok(result) => {
                let playbacks = result.data;
                let has_more = match &result.paginator {
                    Some(p) => page + 1 < p.pages,
                    None => playbacks.len() as i32 == PULL_PAGE_SIZE,
                };
                self.merge_history_page(playbacks).await;
                self.history.history_has_more = has_more;
                self.history.history_next_page = page + 1;
                debug!(page, has_more, "Loaded history playback page");
            }
            Err(e) => {
                // Stop paging until the next reset (refresh); the cached overlay is
                // all History can show right now.
                warn!(error = %e, "History playback page fetch failed");
                self.history.history_has_more = false;
            }
        }
        self.publish();
    }

    /// Merge one fetched `/playbacks` page into the `playbacks` overlay + local
    /// store, skipping rows whose episode has a queued `SetCursor`/`MarkPlayed`
    /// outbox op: the local optimistic row is NEWER than anything the server can
    /// return until the op drains, so overwriting it would visibly regress the
    /// resume position (and the History order) until the next drain. Mirrors the
    /// pending-op facet guard in `cache_episodes`. Split from [`Self::load_history`]
    /// so the merge rule is testable without a live server.
    pub(super) async fn merge_history_page(&mut self, playbacks: Vec<PlaybackData>) {
        let mut playback_pending: HashSet<i32> = HashSet::new();
        if let Ok(pending) = self.store.pending().await {
            for (_, op) in &pending {
                match op {
                    OutboxOp::SetCursor { episode_id, .. }
                    | OutboxOp::MarkPlayed { episode_id, .. } => {
                        playback_pending.insert(*episode_id);
                    }
                    _ => {}
                }
            }
        }
        for pb in &playbacks {
            if playback_pending.contains(&pb.episode_id) {
                continue;
            }
            if let Err(e) = self.store.save_playback(pb).await {
                error!(error = %e, "Failed to persist history playback page");
            }
            self.playbacks.playbacks.insert(pb.episode_id, pb.clone());
        }
    }

    /// Cross-tab Web Lock name guarding this account's device download of
    /// `episode_id` (see `device_download_task`).
    pub(super) fn download_lock_name(&self, episode_id: i32) -> String {
        format!("halogen.download.{}.{episode_id}", self.lock_scope)
    }

    /// Drain pending outbox operations to the server via [`OutboxOp::apply`].
    pub(super) async fn drain_outbox(&mut self) {
        if self.manual_offline {
            // "Go Offline": hold every queued op until the user reconnects.
            return;
        }
        if self.api_client.is_none() {
            // Unconfigured: keep ops queued until SetAuth.
            return;
        }
        // Web: serialize drains across tabs/PWA windows. Every tab runs its own
        // sync worker over the SAME origin-scoped outbox, so two unserialized
        // drains both read `pending` before either acks and double-apply every
        // op (an `AddToPlaylist` lands twice). The lock is held for the whole
        // drain and MUST be taken before the `pending` read so the loser
        // re-reads after the winner's acks. Auto-released if this tab dies
        // mid-drain; `None` (no Web Locks API) proceeds unguarded — the
        // pre-lock status quo. Native has one process per store: no lock.
        #[cfg(target_arch = "wasm32")]
        let _drain_lock =
            halogen_ui_platform::weblock::acquire(&format!("halogen.outbox.{}", self.lock_scope))
                .await;
        let pending = match self.store.pending().await {
            Ok(ops) => ops,
            Err(e) => {
                error!(error = %e, "Failed to read outbox");
                // Don't assert a connectivity state we didn't actually probe.
                return;
            }
        };

        // Nothing queued: an empty drain performs no network I/O, so it proves
        // nothing about connectivity — leave `sync_status` as the pull probe set
        // it. (Previously this forced `Online`, masking a real Offline: the loop
        // drains after every command, so a playback cursor save or a `/latest`
        // page-cache `CacheEpisodes` would flip Offline straight back to Online.)
        if pending.is_empty() {
            return;
        }

        // A head op that keeps failing with a *countable* error backs off: skip
        // this drain while its budget says wait, so the drain-after-every-command
        // cadence can't hammer (and re-toast) a deterministically-failing op.
        // A different head means the old entry is stale — drop it for a fresh
        // budget. Transport failures never reach this bookkeeping (see below).
        if let Some(head_id) = pending.first().map(|(id, _)| *id) {
            match &mut self.head_retry {
                Some(hr) if hr.op_id == head_id && hr.skip_drains > 0 => {
                    hr.skip_drains -= 1;
                    return;
                }
                Some(hr) if hr.op_id != head_id => self.head_retry = None,
                _ => {}
            }
        }

        self.connection.sync_status = halogen_ui_appstate::state::SyncStatus::Syncing {
            phase: halogen_ui_appstate::state::SyncPhase::Drain,
        };

        let mut needs_pull = false;
        let mut failures = 0u32;
        let mut dropped = 0u32;
        // Classify failures inside the loop (free fn, no `self` borrow); apply the
        // side effects after the `api` borrow ends.
        let mut decisions: Vec<ToastDecision> = Vec::new();
        // Podcasts returned by drained Subscribe ops — merged into the pool below
        // so the new subscription shows up without waiting for a list re-fetch.
        let mut subscribed: Vec<halogen_wire::PodcastData> = Vec::new();
        {
            let api = self.api_client.as_ref().expect("checked above");
            for (op_id, op) in &pending {
                debug!(op_id, op = ?op, "Draining outbox op");
                match op.apply(api).await {
                    Ok(created) => {
                        subscribed.extend(created);
                        needs_pull |= op.needs_pull_after();
                        // A tracked head that finally succeeded ends its budget.
                        if self.head_retry.as_ref().is_some_and(|h| h.op_id == *op_id) {
                            self.head_retry = None;
                        }
                        if let Err(e) = self.store.ack(*op_id).await {
                            error!(error = %e, "Failed to ack outbox op");
                        }
                    }
                    Err(e) => {
                        decisions.push(classify(&e, ToastPolicy::Background));
                        if is_permanent_failure(&e) {
                            // The server will never accept this op (rejected /
                            // invalid). Dead-letter it (ack to drop) so it stops
                            // re-toasting and blocking the head of the FIFO every
                            // drain; the toast above already told the user.
                            warn!(error = %e, op = ?op, "Outbox op permanently rejected; dropping");
                            if self.head_retry.as_ref().is_some_and(|h| h.op_id == *op_id) {
                                self.head_retry = None;
                            }
                            if let Err(e) = self.store.ack(*op_id).await {
                                error!(error = %e, "Failed to drop dead-lettered outbox op");
                            }
                            dropped += 1;
                        } else if is_countable_failure(&e) {
                            // Countable (5xx / decode / empty): could equally be a
                            // deterministic server bug on this payload or version
                            // skew — retried, but against a budget, else one
                            // poisoned op blocks everything queued behind it
                            // forever while re-toasting every drain.
                            let attempt = match &self.head_retry {
                                Some(h) if h.op_id == *op_id => h.failures + 1,
                                _ => 1,
                            };
                            if attempt >= OUTBOX_DEAD_LETTER_ATTEMPTS {
                                warn!(
                                    error = %e, op = ?op, attempts = attempt,
                                    "Outbox op failed every budgeted retry; dropping"
                                );
                                self.head_retry = None;
                                if let Err(e) = self.store.ack(*op_id).await {
                                    error!(error = %e, "Failed to drop dead-lettered outbox op");
                                }
                                dropped += 1;
                                decisions.push(ToastDecision::Toast(
                                    ToastLevel::Error,
                                    "A queued change kept failing to sync and was dropped."
                                        .to_string(),
                                ));
                            } else {
                                failures += 1;
                                warn!(
                                    error = %e, op = ?op, attempt,
                                    "Outbox op failed; will retry with backoff"
                                );
                                self.head_retry = Some(HeadRetry {
                                    op_id: *op_id,
                                    failures: attempt,
                                    skip_drains: drain_skips(attempt),
                                });
                                // Same head-of-FIFO ordering rule as below.
                                break;
                            }
                        } else {
                            // Transient (offline / throttled / auth) — leave queued
                            // for the next drain, unbudgeted: an offline queue must
                            // retry indefinitely, that's the local-first contract.
                            failures += 1;
                            warn!(error = %e, op = ?op, "Outbox op failed; will retry");
                            // Stop draining at the FIFO head's first transient failure:
                            // acking a later, dependent op while this one stays queued
                            // replays them out of order next drain (e.g. a redownload's
                            // RemoveServerDownload must reach the server before its
                            // TriggerDownload). Permanent failures above ack + continue.
                            break;
                        }
                    }
                }
            }
        }
        if dropped > 0 {
            warn!(dropped, "Dropped permanently-rejected outbox op(s)");
        }
        // Merge freshly-subscribed podcasts into the store + pool (publishes),
        // so /podcasts reflects the subscription the moment the drain lands.
        if !subscribed.is_empty() {
            self.cache_podcasts(subscribed).await;
        }

        // Surface each failure (toast / sign-out) and track connectivity.
        let went_offline = decisions
            .iter()
            .any(|d| matches!(d, ToastDecision::Offline));
        for decision in decisions {
            self.apply_decision(decision);
        }

        if failures > 0 {
            self.connection.last_error =
                Some(format!("{failures} pending action(s) failed to sync"));
        }
        // A new/removed subscription needs a pull to materialize episodes; let it
        // set its own status (Online on success, Offline on failure).
        if needs_pull {
            self.do_pull().await;
            return;
        }
        // A transport failure on any op corroborates offline; otherwise online.
        // (The WS driver is the primary connectivity signal; this keeps an HTTP
        // drain consistent with it.)
        if went_offline {
            self.set_offline_status();
        } else {
            self.set_online_status();
        }
    }

    pub(super) async fn set_auth(&mut self, server_url: String, token: String) {
        match server_url.parse::<Url>() {
            Ok(url)
                if halogen_utils::constants::ALLOWED_SERVER_URL_SCHEMES.contains(&url.scheme()) =>
            {
                // Credentials live in `ClientConfig` (the persisted source of
                // truth the UI's URL builders read); the worker only needs them to
                // build its api client + WS ticket, straight from the params.
                let client = ApiClient::new(url);
                client.set_token(Some(token));
                self.api_client = Some(client);
                // Bring up the connectivity WebSocket with the fresh token (a no-op
                // when manually offline). It owns the green/yellow/red status; the
                // pull below still corroborates reachability and resolves the queue.
                self.start_ws();
                // Ship queued ops BEFORE the pull: `do_pull` funnels the default
                // playlist through `cache_playlists`, whose server `episode_ids`
                // overwrite `episodes_by_playlist` — wiping an offline-queued episode
                // before its `AddToPlaylist` op drains. Draining first replays the
                // membership to the server, so the pull reads it back. `do_pull` is
                // still the authoritative queue resolver (its GET /playlists/default
                // doubles as the offline probe), so the queue resolves here on start.
                // `coalesced_pull`: this is the first of the boot trigger burst, so it
                // runs; the WsConnected/EnsureDefaultPlaylist that follow within a
                // couple seconds collapse onto it.
                self.drain_outbox().await;
                self.coalesced_pull().await;
                // Now that we're authed + (probably) online, resume any device
                // downloads a previous session left half-finished.
                self.resume_partial_downloads().await;
                self.publish();
            }
            Ok(_) => {
                warn!(
                    server_url,
                    "Disallowed server URL scheme in SetAuth; staying idle"
                );
            }
            Err(e) => {
                warn!(error = %e, server_url, "Invalid server URL in SetAuth; staying idle");
            }
        }
    }
}

/// Whether a pull stamped at `last` (epoch-ms, `None` = never) is recent enough at
/// `now` to coalesce a redundant startup pull onto it. Pure so the recency/coalesce
/// rule is unit-testable without a clock or a live `SyncService`. A backwards wall-
/// clock jump (`now < last`) counts as NOT recent, so a stepped-back clock can only
/// ever cause a harmless extra refresh — never wedge a needed pull off for the
/// window (the safe direction, since the worker can't use a monotonic clock here).
fn is_recent(last: Option<u64>, now: u64, window_ms: u64) -> bool {
    last.is_some_and(|t| now >= t && now - t < window_ms)
}

#[cfg(test)]
mod pull_coalesce_tests {
    use super::is_recent;

    #[test]
    fn never_pulled_is_not_recent() {
        assert!(!is_recent(None, 10_000, 2_000));
    }

    #[test]
    fn within_window_is_recent_boundary_exclusive() {
        // Just inside the window coalesces; exactly at the edge does not (the next
        // trigger past the window legitimately re-pulls).
        assert!(is_recent(Some(9_000), 10_999, 2_000));
        assert!(is_recent(Some(9_000), 9_000, 2_000)); // same instant
        assert!(!is_recent(Some(9_000), 11_000, 2_000)); // == window edge
        assert!(!is_recent(Some(9_000), 20_000, 2_000)); // well past (periodic pull)
    }

    #[test]
    fn backwards_clock_jump_is_not_recent() {
        // now < last (wall clock stepped back) always re-pulls — never suppressed —
        // so a bad clock can't wedge a needed startup pull off for the window.
        assert!(!is_recent(Some(10_000), 9_999, 2_000)); // tiny step back
        assert!(!is_recent(Some(10_000), 1_000, 2_000)); // big step back
    }
}

#[cfg(test)]
mod history_merge_tests {
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

    fn pb(episode_id: i32, cursor: u64) -> PlaybackData {
        let now = Utc::now();
        PlaybackData {
            id: 0,
            user_id: 0,
            episode_id,
            cursor,
            completed: false,
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn history_page_skips_rows_with_pending_playback_ops() {
        let mut svc = test_service("history-guard");
        block_on(async {
            // Optimistic local cursor at 900 (what the SetCursor command handler
            // does): overlay + store row written, the op queued but not drained.
            svc.set_playback_locally(5, |p| p.cursor = 900).await;
            svc.enqueue(OutboxOp::SetCursor {
                episode_id: 5,
                cursor: 900,
            })
            .await;

            // A `/playbacks` page fetched before the drain still carries the old
            // cursor for episode 5; episode 6 has no queued op and merges normally.
            svc.merge_history_page(vec![pb(5, 100), pb(6, 50)]).await;

            assert_eq!(
                svc.playbacks.playbacks.get(&5).map(|p| p.cursor),
                Some(900),
                "a stale history page must not regress a cursor with a queued SetCursor"
            );
            assert_eq!(svc.playbacks.playbacks.get(&6).map(|p| p.cursor), Some(50));
            // The persisted rows agree with the overlay.
            let stored = svc.store.list_playbacks().await.unwrap();
            let cursor_of = |id: i32| stored.iter().find(|p| p.episode_id == id).map(|p| p.cursor);
            assert_eq!(cursor_of(5), Some(900));
            assert_eq!(cursor_of(6), Some(50));

            // Once the op drains (acked), the server row merges normally again.
            for (op_id, _) in svc.store.pending().await.unwrap() {
                svc.store.ack(op_id).await.unwrap();
            }
            svc.merge_history_page(vec![pb(5, 100)]).await;
            assert_eq!(svc.playbacks.playbacks.get(&5).map(|p| p.cursor), Some(100));
        });
    }
}

// Outbox head-retry behavior against a real (loopback) HTTP server scripted to
// fail deterministically — the point is the drain's budget/backoff reaction to
// a server that keeps rejecting one op, which no pure-function test can cover.
// Native-only: the drain runs identically on both targets, and the harness
// needs tokio + a TCP listener (mirrors `download::tests`).
#[cfg(all(test, not(target_arch = "wasm32")))]
mod outbox_drain_tests {
    use std::rc::Rc;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    use halogen_ui_svc_store::NativeLocalStore;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    use super::*;
    use crate::SyncService;

    /// A service over a fresh throwaway SQLite store (no media). The events
    /// receiver is dropped — `publish`/toasts tolerate a closed channel.
    fn test_service(db: &str) -> SyncService {
        let path =
            std::env::temp_dir().join(format!("halogen-drain-test-{}-{db}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let store = Rc::new(NativeLocalStore::open(path).expect("open test store"));
        let (tx, _rx) = futures::channel::mpsc::unbounded();
        SyncService::new(store, None, tx)
    }

    fn service_with_api(db: &str, base: Url) -> SyncService {
        let mut svc = test_service(db);
        let client = ApiClient::new(base);
        client.set_token(Some("test-token".to_string()));
        svc.api_client = Some(client);
        svc
    }

    /// Loopback server answering EVERY request with 500 after reading it fully
    /// (a cut-short read would surface as `Transport`, not the 500 under test);
    /// returns its base URL + the count of requests that reached it.
    async fn spawn_500_server() -> (Url, Arc<AtomicU32>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let base =
            Url::parse(&format!("http://{}", listener.local_addr().expect("addr"))).expect("base");
        let hits = Arc::new(AtomicU32::new(0));
        let counter = hits.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    break;
                };
                let counter = counter.clone();
                tokio::spawn(async move {
                    let mut buf = [0u8; 2048];
                    let mut req = Vec::new();
                    let head_end = loop {
                        match sock.read(&mut buf).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => req.extend_from_slice(&buf[..n]),
                        }
                        if let Some(p) = req.windows(4).position(|w| w == b"\r\n\r\n") {
                            break p + 4;
                        }
                    };
                    let head = String::from_utf8_lossy(&req[..head_end]).into_owned();
                    let body_len = head
                        .lines()
                        .find_map(|l| {
                            let (name, v) = l.split_once(':')?;
                            name.trim()
                                .eq_ignore_ascii_case("content-length")
                                .then(|| v.trim().parse::<usize>().ok())
                                .flatten()
                        })
                        .unwrap_or(0);
                    let mut got = req.len() - head_end;
                    while got < body_len {
                        match sock.read(&mut buf).await {
                            Ok(0) | Err(_) => break,
                            Ok(n) => got += n,
                        }
                    }
                    counter.fetch_add(1, Ordering::SeqCst);
                    let _ = sock
                        .write_all(
                            b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                        )
                        .await;
                });
            }
        });
        (base, hits)
    }

    #[tokio::test]
    async fn deterministic_5xx_head_backs_off_then_dead_letters() {
        let (base, hits) = spawn_500_server().await;
        let mut svc = service_with_api("deadletter", base);
        svc.store
            .enqueue(&OutboxOp::MarkPlayed {
                episode_id: 1,
                played: true,
            })
            .await
            .expect("enqueue");

        // First drain: one attempt, op kept queued, budget armed.
        svc.drain_outbox().await;
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        assert_eq!(svc.store.pending().await.expect("pending").len(), 1);

        // Backoff: the immediately-following drain must NOT re-hit the server —
        // pre-fix, the drain-after-every-command loop re-sent it every time.
        svc.drain_outbox().await;
        assert_eq!(hits.load(Ordering::SeqCst), 1, "backoff skipped the retry");

        // Keep draining: the budget must exhaust and dead-letter the op instead
        // of wedging the FIFO forever.
        for _ in 0..200 {
            svc.drain_outbox().await;
            if svc.store.pending().await.expect("pending").is_empty() {
                break;
            }
        }
        assert!(
            svc.store.pending().await.expect("pending").is_empty(),
            "op dead-lettered after budget exhaustion"
        );
        assert_eq!(
            hits.load(Ordering::SeqCst),
            OUTBOX_DEAD_LETTER_ATTEMPTS,
            "exactly the budgeted attempts reached the server"
        );
        assert!(svc.head_retry.is_none());
    }

    #[tokio::test]
    async fn transport_failure_retries_forever_unbudgeted() {
        // Bind then drop → nothing listens → connection refused (Transport).
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let base =
            Url::parse(&format!("http://{}", listener.local_addr().expect("addr"))).expect("base");
        drop(listener);
        let mut svc = service_with_api("transport", base);
        svc.store
            .enqueue(&OutboxOp::MarkPlayed {
                episode_id: 1,
                played: true,
            })
            .await
            .expect("enqueue");

        for _ in 0..OUTBOX_DEAD_LETTER_ATTEMPTS + 5 {
            svc.drain_outbox().await;
        }
        assert_eq!(
            svc.store.pending().await.expect("pending").len(),
            1,
            "an offline op must stay queued indefinitely"
        );
        assert!(
            svc.head_retry.is_none(),
            "transport failures never arm the budget"
        );
    }
}
