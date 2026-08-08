//! Background polling service.
//!
//! [`PollingHandle`] owns the scheduled poll task: it wakes every `interval`
//! (the loop cadence, NOT the per-feed fetch rate) and runs an `rss::sync` that
//! gates each podcast by its resolved poll interval, auto-downloads, and enforces
//! retention. Each tick also runs a download-recovery pass
//! (`download::recover_downloads`): it resets stuck `Downloading` rows back to
//! `DownloadError`, retires attempt-exhausted ones as terminal `DownloadBroken`,
//! and re-attempts the rest. The handle is start/stop/reset-able from the control
//! endpoints.
//! `poll()` and [`spawn_poll_job`](PollingHandle::spawn_poll_job) are the
//! on-demand paths — both force a fetch (ignore intervals); the latter streams
//! per-podcast progress into the [`jobs`] tracker.

pub mod jobs;

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, Utc};
use halogen_wire::{PollJobStatus, PollJobTrigger};
use sea_orm::DatabaseConnection;
use tokio::sync::oneshot;
use tracing::{info, warn};

use self::jobs::JobTracker;
use halogen_download::DownloadTracker;
use halogen_rss::SyncContext;

/// Run one reported feed sync against `ctx`, streaming per-podcast outcomes into
/// the persisted job `job_id` (when `Some`) and finishing it with the run's
/// status. The rss reporter callback is sync, while persisting an outcome is
/// async — a channel + writer task bridges the two; the writer is drained before
/// the job is finished so no outcome lands after `completed_at`.
async fn run_reported_sync(
    dbc: &DatabaseConnection,
    ctx: &SyncContext,
    tracker: &Arc<JobTracker>,
    job_id: Option<u64>,
    podcast_ids: Option<Vec<i32>>,
) -> Result<(), String> {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let writer = match job_id {
        Some(jid) => {
            let tracker = tracker.clone();
            let mut rx = rx;
            Some(tokio::spawn(async move {
                while let Some(result) = rx.recv().await {
                    tracker.record_podcast(jid, result).await;
                }
            }))
        }
        // No job row (its insert failed) — the sync still runs; sends just fail
        // on the closed channel and are ignored.
        None => {
            drop(rx);
            None
        }
    };
    let result = halogen_rss::sync_reported_with_context(dbc, ctx, podcast_ids, move |r| {
        let _ = tx.send(r);
    })
    .await;
    if let Some(writer) = writer {
        let _ = writer.await;
    }
    if let Some(jid) = job_id {
        let status = match &result {
            Ok(()) => PollJobStatus::Completed,
            Err(_) => PollJobStatus::Failed,
        };
        tracker.finish(jid, status).await;
    }
    result
}

struct PollingTask {
    shutdown_tx: oneshot::Sender<()>,
    running: Arc<AtomicBool>,
    /// The spawned loop task, kept so [`PollingHandle::shutdown`] can await/abort
    /// real termination (a plain [`stop`](PollingHandle::stop) only signals).
    join: tokio::task::JoinHandle<()>,
}

impl fmt::Debug for PollingTask {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PollingTask")
            .field("running", &self.running.load(Ordering::SeqCst))
            .finish()
    }
}

#[derive(Clone)]
pub struct PollingHandle {
    dbc: DatabaseConnection,
    /// How often the poller WAKES to scan for due podcasts (not how often any one
    /// feed is fetched — that's `base_ctx.fallback_poll_interval` / the per-podcast
    /// value).
    interval: Arc<Mutex<Duration>>,
    task: Arc<Mutex<Option<PollingTask>>>,
    /// The resolved sync knobs (cutoff, concurrency, fallbacks, auto-download,
    /// media root, …) minus the per-call `respect_poll_interval` flag that
    /// [`sync_context`](Self::sync_context) sets. These were eight separate fields
    /// that, together, simply *were* a [`SyncContext`].
    base_ctx: SyncContext,
    /// DB-backed history of poll jobs (manual + scheduled, capped). Shared so a
    /// spawned poll task can stream per-podcast results into it while the
    /// handlers read.
    jobs: Arc<JobTracker>,
    /// In-memory progress for in-flight downloads. Single source: also handed to
    /// `base_ctx.tracker` (worker/recovery path) and sourced for the manual
    /// download path, so the API and both download callers share one tracker.
    download_tracker: Arc<DownloadTracker>,
}

impl fmt::Debug for PollingHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PollingHandle")
            .field(
                "interval",
                &*self.interval.lock().unwrap_or_else(|_| panic!("poisoned")),
            )
            .field(
                "task",
                &*self.task.lock().unwrap_or_else(|_| panic!("poisoned")),
            )
            .finish()
    }
}

impl PollingHandle {
    pub fn new(dbc: DatabaseConnection, interval: Duration, max_concurrent: usize) -> Self {
        let download_tracker = Arc::new(DownloadTracker::new());
        let jobs = Arc::new(JobTracker::new(dbc.clone()));
        Self {
            dbc,
            interval: Arc::new(Mutex::new(interval)),
            task: Arc::new(Mutex::new(None)),
            // Defaults preserve the legacy "poll everything every wake, no
            // auto-download" behaviour until `with_subscription_config` wires the
            // real `Config` (production via `main`). `respect_poll_interval` is a
            // placeholder — `sync_context` overrides it per call.
            base_ctx: SyncContext {
                max_poll_concurrent: max_concurrent,
                // The shared tracker (not Default's fresh one) so the manual
                // download path and the progress API observe the same in-flight set.
                tracker: download_tracker.clone(),
                ..SyncContext::default()
            },
            jobs,
            download_tracker,
        }
    }

    /// Build the handle the way a full server host does — shared by the
    /// `halogen-server` binary and the embedded in-process server: the wake
    /// interval (ZERO when the polling service is disabled — the loop never
    /// fires), poll concurrency, the UTC-midnight `no_sync_before` cutoff, and
    /// the resolved subscription knobs.
    pub fn from_config(dbc: DatabaseConnection, cfg: &halogen_config::Config) -> Self {
        // Episodes published before this instant are skipped by the sync
        // service (UTC midnight of the configured `subscription_no_sync_before`
        // day).
        let no_sync_before = cfg
            .subscription_no_sync_before
            .and_hms_opt(0, 0, 0)
            .expect("midnight is a valid time")
            .and_utc();

        let wake_interval = if cfg.server_disable_polling_service {
            Duration::ZERO
        } else {
            cfg.subscription_poll_wake_interval
        };

        Self::new(dbc, wake_interval, cfg.subscription_max_poll_concurrent)
            .with_no_sync_before(Some(no_sync_before))
            .with_subscription_config(cfg)
    }

    /// Apply the subscription knobs from the resolved [`Config`](halogen_config::Config):
    /// per-podcast fallbacks, auto-download, retention, media root. Builder so the
    /// `new(...)` call sites in tests stay unchanged.
    pub fn with_subscription_config(mut self, cfg: &halogen_config::Config) -> Self {
        self.base_ctx.fallback_poll_interval = cfg.subscription_fallback_poll_interval;
        self.base_ctx.fallback_max_episodes = cfg.subscription_fallback_max_episodes;
        self.base_ctx.max_concurrent_downloads = cfg.subscription_max_concurrent_downloads;
        self.base_ctx.auto_download_enabled = cfg.subscription_poll_auto_download_enabled;
        self.base_ctx.auto_playlist_add_to_start = cfg.subscription_auto_playlist_add_to_start;
        self.base_ctx.media_root = cfg.media_root.clone();
        self.base_ctx.use_mock_download = cfg.dev_use_mock_download;
        self.base_ctx.download_stuck_after = cfg.subscription_download_stuck_after;
        self.base_ctx.download_max_attempts = cfg.subscription_download_max_attempts;
        self
    }

    /// Build a [`SyncContext`](halogen_rss::SyncContext) from the handle's resolved
    /// knobs. `respect_poll_interval` is `true` for the scheduled wake (gate each
    /// podcast by its interval) and `false` for explicit/manual polls (force).
    fn sync_context(&self, respect_poll_interval: bool) -> SyncContext {
        SyncContext {
            respect_poll_interval,
            ..self.base_ctx.clone()
        }
    }

    /// Shared poll-job history, for the `/poll-job*` handlers to read.
    pub fn jobs(&self) -> Arc<JobTracker> {
        self.jobs.clone()
    }

    /// Shared in-flight download progress tracker, for the download-progress
    /// handlers and the manual download path to read/write.
    pub fn download_tracker(&self) -> Arc<DownloadTracker> {
        self.download_tracker.clone()
    }

    /// Start an on-demand poll job and return its id once the job row is
    /// persisted. The actual feed sync runs on a detached task, streaming
    /// per-podcast results into the job history; clients poll
    /// `GET /admin/poll-job/{id}` for progress. `podcast_id` scopes the run to
    /// one feed (`None` = all feeds).
    pub async fn spawn_poll_job(&self, podcast_id: Option<i32>) -> Result<u64, String> {
        let job_id = self.jobs.create(PollJobTrigger::Manual, podcast_id).await?;
        let dbc = self.dbc.clone();
        // Explicit poll → force a fetch regardless of each podcast's interval.
        let ctx = self.sync_context(false);
        let tracker = self.jobs.clone();

        tokio::spawn(async move {
            let ids = podcast_id.map(|id| vec![id]);
            if let Err(e) = run_reported_sync(&dbc, &ctx, &tracker, Some(job_id), ids).await {
                warn!("Poll job {job_id} failed: {e}");
            }
        });

        Ok(job_id)
    }

    /// Set the "ignore episodes published before this day" cutoff that the sync
    /// service applies. Builder so existing `new(...)` call sites are unaffected.
    pub fn with_no_sync_before(mut self, cutoff: Option<DateTime<Utc>>) -> Self {
        self.base_ctx.no_sync_before = cutoff;
        self
    }

    pub fn start(&self) -> Result<(), String> {
        let mut task = self
            .task
            .lock()
            .map_err(|_| "Polling handle lock poisoned".to_string())?;

        if task.is_some() {
            return Err("Polling service is already running".to_string());
        }

        let running = Arc::new(AtomicBool::new(true));

        let (shutdown_tx, mut shutdown_rx) = oneshot::channel();

        let dbc = self.dbc.clone();
        let interval = *self
            .interval
            .lock()
            .map_err(|_| "Polling interval lock poisoned".to_string())?;
        // Scheduled wake → gate each podcast by its resolved poll interval.
        let ctx = self.sync_context(true);
        let tracker = self.jobs.clone();

        let running_clone = running.clone();

        let join = tokio::spawn(async move {
            if interval.is_zero() {
                info!("Polling interval is zero, skipping polling service");
                return;
            }

            info!("Starting polling service with interval: {:?}", interval);

            let mut ticker = tokio::time::interval(interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            ticker.tick().await;

            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => {
                        info!("Polling service received shutdown signal");
                        break;
                    }
                    _ = ticker.tick() => {
                        info!("Polling service running sync");
                        // Persist the tick as a scheduled job so its status +
                        // per-podcast outcomes are queryable like a manual run.
                        // Best-effort: the sync itself runs even if the job row
                        // couldn't be written.
                        let job_id = match tracker
                            .create(PollJobTrigger::Scheduled, None)
                            .await
                        {
                            Ok(id) => Some(id),
                            Err(e) => {
                                warn!("Failed to persist scheduled poll job: {e}");
                                None
                            }
                        };
                        match run_reported_sync(&dbc, &ctx, &tracker, job_id, None).await {
                            Ok(()) => {
                                info!("Polling service sync completed successfully");
                            }
                            Err(e) => {
                                warn!("Polling service sync failed: {}", e);
                            }
                        }
                        // Self-heal stuck / errored downloads (watchdog + bounded
                        // retry + broken-cap). Best-effort; never aborts the tick.
                        halogen_download::recover_downloads(
                            &dbc,
                            &ctx.download_options(),
                            ctx.download_stuck_after,
                            ctx.download_max_attempts,
                            ctx.max_concurrent_downloads,
                        )
                        .await;
                    }
                }
            }

            running_clone.store(false, Ordering::SeqCst);
            info!("Polling service stopped");
        });

        *task = Some(PollingTask {
            shutdown_tx,
            running,
            join,
        });

        info!("Polling service started");
        Ok(())
    }

    pub fn stop(&self) -> Result<(), String> {
        let mut task = self
            .task
            .lock()
            .map_err(|_| "Polling handle lock poisoned".to_string())?;

        let poll_task = task
            .take()
            .ok_or_else(|| "Polling service is not running".to_string())?;

        if poll_task.shutdown_tx.send(()).is_err() {
            warn!("Polling service shutdown channel already dropped");
        }

        info!("Polling service stop signal sent");
        Ok(())
    }

    /// Stop the scheduled poll task and wait for it to actually terminate,
    /// aborting an in-flight tick rather than waiting it out (a tick can spend
    /// minutes in feed fetches). Rows left `Downloading` by an aborted tick are
    /// reclaimed by `reclaim_orphaned_downloads` / the next recovery pass — the
    /// same hardening that covers a process kill. Idempotent: a no-op when the
    /// service isn't running. This is the teardown an in-process host (the
    /// embedded server's restart loop) needs before dropping its DB pool;
    /// [`stop`](Self::stop) alone returns while the old tick may still hold it.
    pub async fn shutdown(&self) {
        // Scope the guard: `std::sync::Mutex` must not be held across an await.
        let poll_task = match self.task.lock() {
            Ok(mut task) => task.take(),
            Err(_) => {
                warn!("Polling handle lock poisoned during shutdown");
                None
            }
        };
        let Some(poll_task) = poll_task else {
            return;
        };
        // Best-effort graceful signal first (covers a task parked in select!),
        // then abort to cut short an in-flight sync at its next await point.
        let _ = poll_task.shutdown_tx.send(());
        poll_task.join.abort();
        // Err(Cancelled) is the expected outcome of the abort.
        let _ = poll_task.join.await;
        info!("Polling service shut down");
    }

    pub fn is_running(&self) -> bool {
        let task = self
            .task
            .lock()
            .unwrap_or_else(|_| panic!("Polling handle lock poisoned"));

        match task.as_ref() {
            Some(t) => t.running.load(Ordering::SeqCst),
            None => false,
        }
    }

    pub async fn poll(&self) -> Result<(), String> {
        info!("Manual poll triggered");
        // Persist the run like every other sync (manual trigger, all feeds).
        // Best-effort: the poll proceeds even if the job row couldn't be written.
        let job_id = match self.jobs.create(PollJobTrigger::Manual, None).await {
            Ok(id) => Some(id),
            Err(e) => {
                warn!("Failed to persist manual poll job: {e}");
                None
            }
        };
        // Manual poll → force a fetch of every podcast (ignore intervals).
        let ctx = self.sync_context(false);
        match run_reported_sync(&self.dbc, &ctx, &self.jobs, job_id, None).await {
            Ok(()) => {
                info!("Manual poll completed successfully");
                Ok(())
            }
            Err(e) => {
                warn!("Manual poll failed: {}", e);
                Err(e)
            }
        }
    }

    pub fn reset_interval(&self, new_interval: Duration) {
        let mut interval = self
            .interval
            .lock()
            .expect("Polling interval lock poisoned");
        *interval = new_interval;
        info!("Polling interval reset to: {:?}", new_interval);
    }
}

#[cfg(test)]
mod tests;
