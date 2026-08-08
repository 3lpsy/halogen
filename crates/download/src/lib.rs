//! Episode media download + server-side storage.
//!
//! [`download_episode`] streams an episode's enclosure (or copies a bundled
//! fixture clip when `use_mock_download`) into `media_root` via a temp file +
//! atomic rename, recording the path, size, and `Downloaded` status. **Transient**
//! failures (network/timeout/5xx/429/408/400/truncation) retry in-call with
//! exponential backoff ([`RetryPolicy`]); 403/404 are terminal
//! (`DownloadUnauthorized` / `DownloadRemoteNotFound`) and any other 4xx fails as
//! `DownloadError` (classified by `DownloadFailure` / `classify_status`). Live
//! byte progress is published through the shared [`DownloadTracker`] (see
//! [`tracker`]) while the stream runs.
//!
//! [`remove_server_download`] reverses a download, and [`enforce_retention`] caps
//! how many downloads a podcast keeps (oldest by `downloaded_at` purged first).
//! [`recover_downloads`] is the per-poll-tick watchdog: it resets rows stuck
//! `Downloading` past the configured cutoff back to `DownloadError`, flips rows
//! that exhausted the whole-call attempt budget (`download_attempts`) to terminal
//! `DownloadBroken`, then re-attempts the rest. Shared by the poller's
//! auto-download/recovery pass and the manual `POST /episodes/{id}/download` (and
//! `/episodes/download/bulk`) endpoints.

#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::Duration;

use chrono::{DateTime, Utc};
use futures_util::FutureExt;
use reqwest::{Client, StatusCode};
use sea_orm::sea_query::Expr;
use sea_orm::{
    ColumnTrait, Condition, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder, QuerySelect,
    Set,
};
use tracing::{info, warn};

use halogen_orm::episode::{
    ActiveModel as EpisodeActiveModel, Column as EpisodeColumn, Entity as EpisodeEntity,
    Model as EpisodeModel,
};
use halogen_utils::constants::MAX_DISK_USAGE_PERCENT;
use halogen_wire::DownloadStatus;
use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;
use tokio::sync::Semaphore;

pub mod tracker;

pub use tracker::{DownloadTracker, ProgressEntry};

// Canonical home is `halogen_utils::constants` (the shared Config defaults), so the
// download service and `halogen-config` agree on one value. Re-exported here so
// `halogen_download::DEFAULT_DOWNLOAD_*` keeps resolving for downstream callers.
pub use halogen_utils::constants::{DEFAULT_DOWNLOAD_MAX_ATTEMPTS, DEFAULT_DOWNLOAD_STUCK_AFTER};

/// Download-level transient retries inside ONE `download_episode` call. These do
/// NOT touch `download_attempts` (which counts whole calls).
const DOWNLOAD_RETRY_ATTEMPTS: u32 = 4;
/// Base backoff between transient retries; doubles each successive try.
const DOWNLOAD_BACKOFF_BASE: Duration = Duration::from_secs(1);
/// Max errored episodes re-attempted per recovery tick, so a large `DownloadError`
/// backlog isn't re-spawned in full every poll. The rest follow on later ticks.
const RECOVERY_BATCH_MAX: u64 = 100;

static DOWNLOAD_CLIENT: OnceLock<Client> = OnceLock::new();

/// Shared HTTP client for media downloads: hour-scale timeouts (episodes are
/// large, origins slow) and **gzip disabled** so the body is byte-exact and
/// `Content-Length` stays trustworthy for the integrity check (the `gzip` feature
/// is on crate-wide for feeds, which DO want transparent decompression). Memoized
/// in a `OnceLock` so the connection pool is reused across episodes instead of
/// rebuilt per download. The art cache deliberately does NOT use it.
pub fn download_client() -> Client {
    DOWNLOAD_CLIENT
        .get_or_init(|| {
            halogen_net::guarded_client_builder()
                // Deployer override (`server_fetch_user_agent`) or the
                // purpose default — see `halogen_net::user_agent` for why an
                // absent UA gets 403d by media CDNs.
                .user_agent(halogen_net::user_agent("podcast-download"))
                .timeout(Duration::from_secs(3600))
                .read_timeout(Duration::from_secs(3600))
                .gzip(false)
                .build()
                .expect("build reqwest client")
        })
        .clone()
}

/// Per-call download dependencies + retry policy, built by each caller (worker via
/// `SyncContext::download_options`, manual route via `MediaDownloadConfig`, tests
/// via a `none()` retry for speed).
#[derive(Clone)]
pub struct DownloadOptions {
    pub media_root: PathBuf,
    pub use_mock_download: bool,
    pub tracker: Arc<DownloadTracker>,
    pub retry: RetryPolicy,
}

/// Download-level transient retry policy (inside one `download_episode` call).
#[derive(Clone, Copy)]
pub struct RetryPolicy {
    pub attempts: u32,
    pub backoff_base: Duration,
}

impl RetryPolicy {
    /// 4 attempts with exponential backoff from 1s — the steady-state policy.
    pub fn production() -> Self {
        Self {
            attempts: DOWNLOAD_RETRY_ATTEMPTS,
            backoff_base: DOWNLOAD_BACKOFF_BASE,
        }
    }

    /// A single attempt, no backoff — used by tests so they never sleep.
    pub fn none() -> Self {
        Self {
            attempts: 1,
            backoff_base: Duration::ZERO,
        }
    }
}

/// Classified download failure: drives both the download-level retry decision and
/// the terminal `download_status` written on final failure.
enum DownloadFailure {
    /// Worth retrying: network blip, timeout, 5xx/429/408/400, truncation.
    Transient(anyhow::Error),
    /// Origin returned 403 — terminal (`DownloadUnauthorized`).
    Unauthorized,
    /// Origin returned 404 — terminal (`DownloadRemoteNotFound`).
    RemoteNotFound,
    /// Non-retryable but not specially classified (other 4xx, etc.) →
    /// `DownloadError`.
    Permanent(anyhow::Error),
}

impl DownloadFailure {
    fn is_transient(&self) -> bool {
        matches!(self, DownloadFailure::Transient(_))
    }

    /// The terminal status to persist for this failure.
    fn terminal_status(&self) -> DownloadStatus {
        match self {
            DownloadFailure::Unauthorized => DownloadStatus::DownloadUnauthorized,
            DownloadFailure::RemoteNotFound => DownloadStatus::DownloadRemoteNotFound,
            DownloadFailure::Transient(_) | DownloadFailure::Permanent(_) => {
                DownloadStatus::DownloadError
            }
        }
    }

    fn into_error(self) -> anyhow::Error {
        match self {
            DownloadFailure::Transient(e) | DownloadFailure::Permanent(e) => e,
            other => anyhow::anyhow!("{other}"),
        }
    }
}

impl std::fmt::Display for DownloadFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DownloadFailure::Transient(e) | DownloadFailure::Permanent(e) => write!(f, "{e}"),
            DownloadFailure::Unauthorized => write!(f, "origin returned 403 (unauthorized)"),
            DownloadFailure::RemoteNotFound => write!(f, "origin returned 404 (not found)"),
        }
    }
}

/// Map an unsuccessful HTTP status onto a failure class. 403/404 are terminal;
/// 400/408/429/5xx are transient (retry); any other 4xx is permanent.
fn classify_status(status: StatusCode) -> DownloadFailure {
    match status {
        StatusCode::FORBIDDEN => DownloadFailure::Unauthorized,
        StatusCode::NOT_FOUND => DownloadFailure::RemoteNotFound,
        s if s.is_server_error()
            || s == StatusCode::BAD_REQUEST
            || s == StatusCode::REQUEST_TIMEOUT
            || s == StatusCode::TOO_MANY_REQUESTS =>
        {
            DownloadFailure::Transient(anyhow::anyhow!("HTTP error: {s}"))
        }
        s => DownloadFailure::Permanent(anyhow::anyhow!("HTTP error: {s}")),
    }
}

static FEED_CLIENT: OnceLock<Client> = OnceLock::new();
static CHAPTERS_CLIENT: OnceLock<Client> = OnceLock::new();

/// HTTP client for polling RSS feeds. Unlike [`download_client`], redirects are
/// NOT auto-followed (`Policy::none`): the RSS manager follows them by hand so it
/// can record the full hop chain into `podcast.feed_url_redirects`. Feeds are
/// small, so a short-ish timeout is fine. Memoized (like [`download_client`]) so
/// the connection pool is reused instead of rebuilt per feed.
pub fn feed_client() -> Client {
    FEED_CLIENT
        .get_or_init(|| {
            halogen_net::guarded_client_builder()
                .user_agent(halogen_net::user_agent("podcast-feed"))
                .timeout(Duration::from_secs(60))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .expect("build reqwest feed client")
        })
        .clone()
}

/// HTTP client for the best-effort `podcast:chapters` JSON fetch during sync.
/// SSRF-guarded like every outbound client; a tight timeout so a slow/hostile
/// chapter host can never stall (or meaningfully slow) feed ingestion. Redirects
/// follow normally — the chapters file commonly lives behind a CDN redirect.
/// Memoized so the per-episode fetch during a sync reuses one connection pool.
pub fn chapters_client() -> Client {
    CHAPTERS_CLIENT
        .get_or_init(|| {
            halogen_net::guarded_client_builder()
                .user_agent(halogen_net::user_agent("podcast-chapters"))
                .timeout(Duration::from_secs(15))
                .build()
                .expect("build reqwest chapters client")
        })
        .clone()
}

pub async fn download_episode(
    dbc: &DatabaseConnection,
    episode_id: i32,
    opts: &DownloadOptions,
) -> anyhow::Result<()> {
    let episode = EpisodeEntity::find()
        .filter(EpisodeColumn::Id.eq(episode_id))
        .one(dbc)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Episode not found"))?;

    let title = episode.title.clone();
    let episode_id = episode.id;
    let download_status = episode.download_status.clone();

    // Only an in-flight or completed download short-circuits. Every error-ish
    // state (incl. the terminal ones) proceeds, so a manual `POST /download` can
    // force a fresh attempt; the auto-recovery loop only feeds `DownloadError` in.
    if download_status == DownloadStatus::Downloading {
        info!("Episode '{}' is already being downloaded, skipping", title);
        return Ok(());
    }
    if download_status == DownloadStatus::Downloaded {
        info!(
            "Episode '{}' already downloaded at {}",
            title,
            episode.content_file_path.as_deref().unwrap_or("unknown")
        );
        return Ok(());
    }

    // Atomically claim the attempt (compare-and-swap): flip to Downloading, stamp
    // the start, bump the counter — but only if nobody else already did. The plain
    // status reads above are a fast path; this is the race-free guard. If we lose
    // the claim (a concurrent trigger won), bail rather than double-fetch.
    if !begin_attempt(dbc, &episode).await? {
        info!(
            "Episode '{}' claimed by a concurrent download, skipping",
            title
        );
        return Ok(());
    }

    let client = download_client();
    // Catch a panic in the fetch so the in-flight bookkeeping is ALWAYS unwound: an
    // uncaught panic would skip the `finish` + `set_failed` below, leaking the
    // tracker entry (the progress API would then 200 forever and the client ring
    // spin forever) and stranding the row in `Downloading` (blocking re-trigger
    // until the multi-hour watchdog). A panic is treated as a transient failure, so
    // the recovery loop retries it like any other `DownloadError`.
    let result = AssertUnwindSafe(fetch_episode(&client, &episode, opts))
        .catch_unwind()
        .await
        .unwrap_or_else(|_| {
            Err(DownloadFailure::Transient(anyhow::anyhow!(
                "download task panicked"
            )))
        });
    // Either way the download is no longer in-flight — drop the progress entry.
    opts.tracker.finish(episode_id);

    let local_path = match result {
        Ok(path) => path,
        Err(failure) => {
            let status = failure.terminal_status();
            let reason = failure.to_string();
            let err = failure.into_error();
            warn!(
                "Download failed for episode '{}': {err}; marking {}",
                title, status
            );
            if let Err(fail_err) = set_failed(dbc, episode_id, status).await {
                warn!(
                    "Failed to set terminal download status for episode '{}': {}",
                    title, fail_err
                );
            }
            // Persist the failure reason — the durable trail behind the admin
            // Server Errors page (the status column only says THAT it failed).
            // Best-effort: a failed write is only logged.
            if let Err(rec_err) =
                halogen_orm::episode_download_error::Entity::record(dbc, episode_id, reason).await
            {
                warn!(
                    "Failed to persist download error for episode '{}': {}",
                    title, rec_err
                );
            }
            return Err(err);
        }
    };

    let now = Utc::now();
    // Record the stored file's size alongside the path + timestamp so the three
    // stay in sync (all cleared together on removal).
    let download_size = tokio::fs::metadata(&local_path)
        .await
        .ok()
        .map(|m| m.len() as i64);
    // Reuse the in-memory row — the atomic claim above made us the sole in-flight
    // writer for this episode (any concurrent trigger lost the CAS and bailed), so
    // no refetch is needed. The final update only `Set`s the four result columns,
    // leaving the `download_attempts` that `begin_attempt` bumped untouched.
    let mut update = EpisodeActiveModel::from(episode);
    update.downloaded_at = Set(Some(now));
    update.content_file_path = Set(Some(local_path));
    update.download_size = Set(download_size);
    update.download_status = Set(DownloadStatus::Downloaded);
    EpisodeEntity::update(update)
        .exec(dbc)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to update episode download status: {e}"))?;

    info!("Episode '{}' downloaded successfully", title);
    Ok(())
}

/// Atomically claim a download attempt with a compare-and-swap on the status: one
/// UPDATE setting `status=Downloading`, `download_started_at=now`,
/// `download_attempts = attempts + 1`, gated `WHERE status NOT IN (Downloading,
/// Downloaded)`. Returns `true` if this call won the claim, `false` if a concurrent
/// trigger already holds it (or it completed) — so the caller bails instead of
/// double-fetching. This atomic gate is the real guard; the plain status reads in
/// `download_episode` are just a fast path. It is the ONE place `download_attempts`
/// increments (per call, first try included); the in-call retry loop never does.
async fn begin_attempt(dbc: &DatabaseConnection, episode: &EpisodeModel) -> anyhow::Result<bool> {
    let res = EpisodeEntity::update_many()
        .col_expr(
            EpisodeColumn::DownloadStatus,
            Expr::value(DownloadStatus::Downloading),
        )
        .col_expr(EpisodeColumn::DownloadStartedAt, Expr::value(Utc::now()))
        .col_expr(
            EpisodeColumn::DownloadAttempts,
            Expr::value(episode.download_attempts + 1),
        )
        .filter(EpisodeColumn::Id.eq(episode.id))
        .filter(
            EpisodeColumn::DownloadStatus
                .is_not_in([DownloadStatus::Downloading, DownloadStatus::Downloaded]),
        )
        .exec(dbc)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to set download status: {e}"))?;
    Ok(res.rows_affected > 0)
}

/// Fetch an episode's media to disk: the bundled mock clip when `use_mock`
/// (falling back to a real fetch if the fixture is missing), else a direct remote
/// download. The caller marks the row errored on `Err`.
async fn fetch_episode(
    client: &Client,
    episode: &halogen_orm::episode::Model,
    opts: &DownloadOptions,
) -> Result<String, DownloadFailure> {
    if opts.use_mock_download {
        match download_mock(episode, &opts.media_root).await {
            Ok(path) => return Ok(path),
            Err(e) => warn!(
                "Mock download failed for episode '{}': {e}, falling back to remote",
                episode.title
            ),
        }
    }
    download_with_retry(client, episode, opts).await
}

/// Run `download_remote` under the download-level retry policy: on a `Transient`
/// failure sleep `backoff_base * 2^try` and retry; on any terminal/permanent
/// failure stop immediately. The tracker is reset to 0 at the start of each try
/// (inside `download_remote`).
async fn download_with_retry(
    client: &Client,
    episode: &halogen_orm::episode::Model,
    opts: &DownloadOptions,
) -> Result<String, DownloadFailure> {
    let attempts = opts.retry.attempts.max(1);
    let mut last: Option<DownloadFailure> = None;
    for attempt in 0..attempts {
        match download_remote(client, episode, &opts.media_root, &opts.tracker).await {
            Ok(path) => return Ok(path),
            Err(failure) => {
                // Terminal/permanent: stop now, don't burn the retry budget.
                if !failure.is_transient() {
                    return Err(failure);
                }
                warn!(
                    "Download attempt {}/{} for episode '{}' failed (transient): {failure}",
                    attempt + 1,
                    attempts,
                    episode.title,
                );
                last = Some(failure);
                if attempt + 1 < attempts {
                    // Saturate the exponent + multiply so an aggressively large
                    // caller-set `attempts`/`backoff_base` can't panic (debug) or
                    // wrap (release) on overflow; production uses 4 attempts.
                    let backoff = opts
                        .retry
                        .backoff_base
                        .saturating_mul(2u32.saturating_pow(attempt.min(16)));
                    if !backoff.is_zero() {
                        tokio::time::sleep(backoff).await;
                    }
                }
            }
        }
    }
    Err(last.unwrap_or_else(|| {
        DownloadFailure::Transient(anyhow::anyhow!("download failed with no attempts run"))
    }))
}

pub(crate) async fn download_mock(
    episode: &halogen_orm::episode::Model,
    media_root: &Path,
) -> anyhow::Result<String> {
    let mock_audio_path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/tests/nasa-test-clip.mp3");

    if !mock_audio_path.exists() {
        anyhow::bail!("Mock audio file not found at {}", mock_audio_path.display());
    }

    let file_name = format!("{}_mock.wav", episode.id);
    let dest = media_root.join(&file_name);
    tokio::fs::create_dir_all(media_root).await?;
    tokio::fs::copy(&mock_audio_path, &dest).await?;
    info!(
        "Copied mock audio for episode '{}': {}",
        episode.title,
        dest.display()
    );
    Ok(dest.to_string_lossy().to_string())
}

async fn download_remote(
    client: &Client,
    episode: &halogen_orm::episode::Model,
    media_root: &Path,
    tracker: &DownloadTracker,
) -> Result<String, DownloadFailure> {
    info!(
        "Downloading episode '{}' from {}",
        episode.title, episode.content_url
    );

    let mut resp = match client.get(&episode.content_url).send().await {
        Ok(resp) => resp,
        Err(e) => {
            // Connection/timeout/request-build errors are worth a retry; anything
            // else (e.g. a bad URL) is permanent.
            return Err(if e.is_timeout() || e.is_connect() || e.is_request() {
                DownloadFailure::Transient(e.into())
            } else {
                DownloadFailure::Permanent(e.into())
            });
        }
    };

    if !resp.status().is_success() {
        return Err(classify_status(resp.status()));
    }

    // `Content-Length` (when advertised) is both the progress total and the
    // integrity yardstick. Gzip is disabled on this client, so it reflects the
    // bytes we'll actually write.
    let total = resp.content_length();
    // Reset/insert the progress entry for this try (each retry begins at 0).
    let entry = tracker.begin(episode.id, total);

    let extension = extract_extension(episode);
    let file_name = format!("{}{extension}", episode.id);
    let dest = media_root.join(&file_name);
    let tmp = make_tmp_path(&dest);

    if let Err(e) = tokio::fs::create_dir_all(media_root).await {
        return Err(DownloadFailure::Transient(e.into()));
    }

    // Refuse to start when the media volume is already at/over the usage limit.
    // Transient: a full disk is an operator-resolvable condition (free space and
    // the next pass succeeds), not the feed's fault.
    if !disk_has_headroom(media_root) {
        return Err(DownloadFailure::Transient(anyhow::anyhow!(
            "insufficient disk space: media root at/above {MAX_DISK_USAGE_PERCENT}% usage"
        )));
    }

    let mut file = match OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&tmp)
        .await
    {
        Ok(file) => file,
        Err(e) => return Err(DownloadFailure::Transient(e.into())),
    };

    let written = match download_loop(&mut resp, &mut file, &entry, media_root).await {
        Ok(written) => written,
        Err(e) => {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(DownloadFailure::Transient(e));
        }
    };

    // Integrity: an advertised length that doesn't match what we wrote means a
    // truncated transfer — retry it (truncation is usually a blip).
    if let Some(total) = total
        && written != total
    {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(DownloadFailure::Transient(anyhow::anyhow!(
            "size mismatch for episode '{}': expected {total} bytes, wrote {written}",
            episode.title
        )));
    }

    if let Err(e) = file.flush().await {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(DownloadFailure::Transient(e.into()));
    }
    drop(file);
    if let Err(e) = tokio::fs::rename(&tmp, &dest).await {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(DownloadFailure::Transient(e.into()));
    }

    info!(
        "Downloaded episode '{}' to {}",
        episode.title,
        dest.display()
    );
    Ok(dest.to_string_lossy().to_string())
}

/// Set an episode to a terminal failure `status` (`DownloadError` /
/// `DownloadUnauthorized` / `DownloadRemoteNotFound` / `DownloadBroken`).
pub async fn set_failed(
    dbc: &DatabaseConnection,
    episode_id: i32,
    status: DownloadStatus,
) -> anyhow::Result<()> {
    let episode = EpisodeEntity::find()
        .filter(EpisodeColumn::Id.eq(episode_id))
        .one(dbc)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Episode not found"))?;

    let mut update = EpisodeActiveModel::from(episode);
    update.download_status = Set(status);
    EpisodeEntity::update(update)
        .exec(dbc)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to set download status: {e}"))?;

    Ok(())
}

/// Download many episodes with at most `max_concurrent` running at once. Shared by
/// the poller's auto-download pass and the error-recovery loop; per-episode errors
/// are logged, never propagated (one bad episode must not stall the batch).
pub async fn download_many(
    dbc: &DatabaseConnection,
    ids: Vec<i32>,
    opts: &DownloadOptions,
    max_concurrent: usize,
) {
    if ids.is_empty() {
        return;
    }

    let semaphore = Arc::new(Semaphore::new(max_concurrent.max(1)));
    let mut handles = Vec::with_capacity(ids.len());
    for id in ids {
        let Ok(permit) = semaphore.clone().acquire_owned().await else {
            break;
        };
        let dbc = dbc.clone();
        let opts = opts.clone();
        handles.push(tokio::spawn(async move {
            let _permit = permit;
            if let Err(e) = download_episode(&dbc, id, &opts).await {
                warn!("download failed for episode {id}: {e}");
            }
        }));
    }
    for h in handles {
        let _ = h.await;
    }
}

/// Reset orphaned `Downloading` rows back to `DownloadError` so they re-enter the
/// recovery retry path.
///
/// `cutoff = None` resets EVERY in-flight row (boot reconcile: right after start
/// nothing can legitimately be downloading). `Some(t)` resets only rows started
/// before `t` or with a NULL start — the steady-state watchdog for same-process
/// zombies and pre-migration rows. Returns the number of rows reset.
pub async fn reset_stuck_downloads(
    dbc: &DatabaseConnection,
    cutoff: Option<DateTime<Utc>>,
) -> anyhow::Result<u64> {
    let mut cond =
        Condition::all().add(EpisodeColumn::DownloadStatus.eq(DownloadStatus::Downloading));
    if let Some(t) = cutoff {
        cond = cond.add(
            Condition::any()
                .add(EpisodeColumn::DownloadStartedAt.is_null())
                .add(EpisodeColumn::DownloadStartedAt.lt(t)),
        );
    }

    let res = EpisodeEntity::update_many()
        .col_expr(
            EpisodeColumn::DownloadStatus,
            Expr::value(DownloadStatus::DownloadError),
        )
        .filter(cond)
        .exec(dbc)
        .await?;

    if res.rows_affected > 0 {
        info!(
            "reset {} stuck download(s) to DownloadError",
            res.rows_affected
        );
    }
    Ok(res.rows_affected)
}

/// Boot reconcile: reset ALL `Downloading` rows to `DownloadError`. Called once at
/// startup (nothing can be mid-download immediately after a restart).
pub async fn reclaim_orphaned_downloads(dbc: &DatabaseConnection) -> anyhow::Result<u64> {
    reset_stuck_downloads(dbc, None).await
}

/// Give up on downloads that have burned their attempt budget: flip
/// `DownloadError` rows with `download_attempts >= max_attempts` to the terminal
/// `DownloadBroken`. Returns the number of rows marked.
async fn mark_broken(dbc: &DatabaseConnection, max_attempts: usize) -> anyhow::Result<u64> {
    let res = EpisodeEntity::update_many()
        .col_expr(
            EpisodeColumn::DownloadStatus,
            Expr::value(DownloadStatus::DownloadBroken),
        )
        .filter(EpisodeColumn::DownloadStatus.eq(DownloadStatus::DownloadError))
        .filter(EpisodeColumn::DownloadAttempts.gte(max_attempts as i32))
        .exec(dbc)
        .await?;

    if res.rows_affected > 0 {
        info!(
            "marked {} download(s) DownloadBroken (>= {max_attempts} attempts)",
            res.rows_affected
        );
    }
    Ok(res.rows_affected)
}

/// Recovery pass run each poller tick: (1) reset stuck `Downloading` rows past the
/// watchdog cutoff, (2) cap exhausted `DownloadError` rows to `DownloadBroken`,
/// (3) re-download whatever `DownloadError` remains. Errors are logged, never
/// propagated — recovery is best-effort and must not abort the tick.
pub async fn recover_downloads(
    dbc: &DatabaseConnection,
    opts: &DownloadOptions,
    stuck_after: Duration,
    max_attempts: usize,
    max_concurrent: usize,
) {
    let cutoff = Utc::now()
        - chrono::Duration::from_std(stuck_after).unwrap_or_else(|_| chrono::Duration::hours(6));
    if let Err(e) = reset_stuck_downloads(dbc, Some(cutoff)).await {
        warn!("download watchdog failed: {e}");
    }

    if let Err(e) = mark_broken(dbc, max_attempts).await {
        warn!("download broken-cap failed: {e}");
    }

    // Cap the per-tick recovery batch: without a limit, a large backlog of errored
    // rows still under the attempt budget would be re-spawned in full every tick
    // (and ticks could overlap if a sync runs long). Least-attempted rows go first
    // so a persistent failure can't starve newer ones; the remainder is picked up
    // on subsequent ticks (and is capped to DownloadBroken once it exhausts attempts).
    let ids = match EpisodeEntity::find()
        .filter(EpisodeColumn::DownloadStatus.eq(DownloadStatus::DownloadError))
        .order_by_asc(EpisodeColumn::DownloadAttempts)
        .order_by_asc(EpisodeColumn::Id)
        .limit(RECOVERY_BATCH_MAX)
        .all(dbc)
        .await
    {
        Ok(rows) => rows.into_iter().map(|e| e.id).collect::<Vec<_>>(),
        Err(e) => {
            warn!("download recovery select failed: {e}");
            return;
        }
    };

    if !ids.is_empty() {
        info!("recovery: re-downloading {} errored episode(s)", ids.len());
        download_many(dbc, ids, opts, max_concurrent).await;
    }
}

/// Remove the server-side downloaded copy of an episode: delete the local file
/// (best-effort) and reset the episode to `NotDownloaded` with no path.
pub async fn remove_server_download(
    dbc: &DatabaseConnection,
    episode_id: i32,
) -> anyhow::Result<()> {
    let episode = EpisodeEntity::find()
        .filter(EpisodeColumn::Id.eq(episode_id))
        .one(dbc)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Episode not found"))?;

    // Delete the stored file if present (ignore a missing file — the DB reset is
    // what matters for the client-visible state).
    if let Some(path) = episode.content_file_path.as_deref() {
        match tokio::fs::remove_file(path).await {
            Ok(()) => info!("Removed server download for episode '{}'", episode.title),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => warn!("Failed to delete file for episode '{}': {e}", episode.title),
        }
    }

    let mut update = EpisodeActiveModel::from(episode);
    update.download_status = Set(DownloadStatus::NotDownloaded);
    update.content_file_path = Set(None);
    update.downloaded_at = Set(None);
    update.download_size = Set(None);
    // Clean slate: a future (re)download starts a fresh attempt budget rather than
    // inheriting the stale count from before removal.
    update.download_started_at = Set(None);
    update.download_attempts = Set(0);
    EpisodeEntity::update(update)
        .exec(dbc)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to reset download status: {e}"))?;

    Ok(())
}

/// Enforce a podcast's server-download retention cap: keep the newest `keep`
/// downloaded episodes and remove the rest (oldest by `downloaded_at` first).
///
/// Counts every server-side download (manual or auto). Called after a download
/// completes — by the poller's auto-download path and the manual download
/// endpoint — so a podcast never holds more than its resolved `max_episodes`.
/// `keep == 0` is treated as "no cap" (purging everything would be surprising and
/// is never what an operator means by leaving the field at its default).
pub async fn enforce_retention(
    dbc: &DatabaseConnection,
    podcast_id: i32,
    keep: usize,
) -> anyhow::Result<usize> {
    if keep == 0 {
        return Ok(0);
    }

    // Newest first; oldest downloads fall to the tail and are the ones purged.
    let downloaded = EpisodeEntity::find()
        .filter(EpisodeColumn::PodcastId.eq(podcast_id))
        .filter(EpisodeColumn::DownloadStatus.eq(DownloadStatus::Downloaded))
        .order_by_desc(EpisodeColumn::DownloadedAt)
        .order_by_desc(EpisodeColumn::Id)
        .all(dbc)
        .await?;

    if downloaded.len() <= keep {
        return Ok(0);
    }

    let mut purged = 0;
    for ep in downloaded.into_iter().skip(keep) {
        remove_server_download(dbc, ep.id).await?;
        purged += 1;
    }
    if purged > 0 {
        info!("Retention purged {purged} old download(s) for podcast '{podcast_id}' (keep {keep})");
    }
    Ok(purged)
}

/// Resolve an episode's podcast retention cap (`podcast_config.max_episodes` ->
/// else `fallback`) and enforce it. Convenience for callers that only hold an
/// episode id (the manual download endpoint).
pub async fn enforce_retention_for_episode(
    dbc: &DatabaseConnection,
    episode_id: i32,
    fallback_max_episodes: usize,
) -> anyhow::Result<usize> {
    use halogen_orm::podcast::{Column as PodCol, Entity as PodEntity};
    use halogen_orm::podcast_config::Entity as CfgEntity;

    let Some(episode) = EpisodeEntity::find()
        .filter(EpisodeColumn::Id.eq(episode_id))
        .one(dbc)
        .await?
    else {
        return Ok(0);
    };
    let podcast_id = episode.podcast_id;

    let keep = match PodEntity::find()
        .filter(PodCol::Id.eq(podcast_id))
        .one(dbc)
        .await?
        .and_then(|p| p.podcast_config_id)
    {
        Some(cfg_id) => CfgEntity::find_by_id(cfg_id)
            .one(dbc)
            .await?
            .and_then(|c| c.max_episodes)
            .map(|v| v as usize)
            .unwrap_or(fallback_max_episodes),
        None => fallback_max_episodes,
    };

    enforce_retention(dbc, podcast_id, keep).await
}

/// Stream the response body to `file`, accumulating into `entry` (lock-free) so
/// the progress API can read live byte counts. Returns total bytes written so the
/// caller can run the `Content-Length` integrity check.
async fn download_loop(
    resp: &mut reqwest::Response,
    file: &mut tokio::fs::File,
    entry: &ProgressEntry,
    media_root: &Path,
) -> anyhow::Result<u64> {
    // Re-check free space this often (by bytes written) so a stream with no/false
    // Content-Length can't run the volume to zero before the pre-check's snapshot
    // goes stale. Coarse enough that statvfs cost is negligible vs. the transfer.
    const DISK_CHECK_INTERVAL_BYTES: u64 = 16 << 20;

    let mut written: u64 = 0;
    let mut since_disk_check: u64 = 0;
    loop {
        let chunk = resp
            .chunk()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to read chunk: {e}"))?;

        match chunk {
            Some(bytes) => {
                file.write_all(&bytes)
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to write chunk: {e}"))?;
                let n = bytes.len() as u64;
                written += n;
                since_disk_check += n;
                entry.add(n);

                if since_disk_check >= DISK_CHECK_INTERVAL_BYTES {
                    since_disk_check = 0;
                    if !disk_has_headroom(media_root) {
                        anyhow::bail!(
                            "aborting download: media root reached {MAX_DISK_USAGE_PERCENT}% disk usage"
                        );
                    }
                }
            }
            None => break,
        }
    }
    Ok(written)
}

/// Whether the filesystem backing `path` is still under [`MAX_DISK_USAGE_PERCENT`]
/// full (used ÷ total capacity). Fails OPEN (returns `true`) when the space
/// query errors or reports a zero-size filesystem: the guard exists to catch a
/// genuinely full volume, and a stat hiccup must not wedge all downloads.
/// `path` must already exist (callers create the media root first).
fn disk_has_headroom(path: &Path) -> bool {
    let (total, avail) = match fs_space(path) {
        Ok(pair) => pair,
        Err(e) => {
            warn!(
                "disk space check for {} failed: {e}; allowing the write",
                path.display()
            );
            return true;
        }
    };
    if total == 0 {
        return true;
    }
    let used = total.saturating_sub(avail);
    // used/total*100 < limit  ⇔  used*100 < total*limit (no floats, no rounding).
    used * 100 < total * MAX_DISK_USAGE_PERCENT as u128
}

/// Total and available-to-this-user bytes of the filesystem backing `path`,
/// as u128 so downstream multiplies can't overflow.
#[cfg(not(windows))]
fn fs_space(path: &Path) -> Result<(u128, u128), String> {
    let stat = rustix::fs::statvfs(path).map_err(|e| e.to_string())?;
    // Bytes = block count × fragment size. `f_bavail` (not `f_bfree`) so root's
    // reserved blocks don't count as headroom for us.
    let frsize = stat.f_frsize as u128;
    Ok((
        stat.f_blocks as u128 * frsize,
        stat.f_bavail as u128 * frsize,
    ))
}

/// Windows: rustix has no `fs` module there, so query `GetDiskFreeSpaceExW`.
/// Its "free bytes available to caller" is quota-aware like `f_bavail`.
#[cfg(windows)]
fn fs_space(path: &Path) -> Result<(u128, u128), String> {
    let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    wide.push(0);
    let mut avail: u64 = 0;
    let mut total: u64 = 0;
    // SAFETY: `wide` is NUL-terminated and outlives the call; both out
    // pointers are valid; the final out param is documented as optional.
    let ok = unsafe {
        windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut avail,
            &mut total,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok((total as u128, avail as u128))
}

fn extract_extension(episode: &halogen_orm::episode::Model) -> String {
    let url = &episode.content_url;
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let name = path.rsplit('/').next().unwrap_or_default();
    name.rsplit_once('.')
        .map(|(_, e)| e)
        .filter(|e| !e.is_empty() && e.len() < 10 && e.chars().all(|c| c.is_ascii_alphanumeric()))
        .map(|e| format!(".{e}").to_lowercase())
        .unwrap_or_else(|| ".wav".into())
}

fn make_tmp_path(dest: &Path) -> PathBuf {
    let suffix = rand::random::<u64>();
    dest.with_extension(format!("tmp.{suffix}"))
}

#[cfg(test)]
mod tests;
