//! Shared types for the RSS sync layer.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use halogen_download::{
    DEFAULT_DOWNLOAD_MAX_ATTEMPTS, DEFAULT_DOWNLOAD_STUCK_AFTER, DownloadOptions, DownloadTracker,
    RetryPolicy,
};

/// Resolved global knobs + per-run flags handed to the sync layer. Per-podcast
/// `podcast_config` values override the fallbacks where present.
#[derive(Clone)]
pub struct SyncContext {
    /// Drop episodes published before this instant while walking the feed.
    pub no_sync_before: Option<DateTime<Utc>>,
    /// Max feeds fetched in parallel during one run.
    pub max_poll_concurrent: usize,
    /// Per-podcast poll interval when the podcast has no override.
    pub fallback_poll_interval: Duration,
    /// Retention cap when the podcast has no override.
    pub fallback_max_episodes: usize,
    /// Parallel auto-downloads per podcast when the podcast has no override.
    pub max_concurrent_downloads: usize,
    /// Auto-download new episodes server-side when the podcast has no override.
    pub auto_download_enabled: bool,
    /// Insert auto-added episodes at the START of their target playlists when
    /// the auto-playlist link has no override (instead of appending at the end).
    pub auto_playlist_add_to_start: bool,
    pub media_root: PathBuf,
    pub use_mock_download: bool,
    /// When true, skip a podcast polled more recently than its resolved interval.
    /// Manual / explicit single-podcast polls pass `false` to force a fetch.
    pub respect_poll_interval: bool,
    /// Shared in-memory progress tracker for in-flight downloads.
    pub tracker: Arc<DownloadTracker>,
    /// A `Downloading` row older than this is treated as orphaned by the watchdog.
    pub download_stuck_after: Duration,
    /// Attempt budget before a failing download is marked `DownloadBroken`.
    pub download_max_attempts: usize,
}

impl Default for SyncContext {
    /// The legacy baseline: poll everything every wake (no interval gating), no
    /// auto-download, no retention cap. Real call sites override only the few
    /// fields they care about via `..Default::default()`; `with_subscription_config`
    /// then wires the resolved `Config` for production.
    fn default() -> Self {
        Self {
            no_sync_before: None,
            max_poll_concurrent: 1,
            fallback_poll_interval: Duration::ZERO,
            fallback_max_episodes: usize::MAX,
            max_concurrent_downloads: 1,
            auto_download_enabled: false,
            auto_playlist_add_to_start: false,
            media_root: PathBuf::from("./media"),
            use_mock_download: false,
            respect_poll_interval: false,
            tracker: Arc::new(DownloadTracker::new()),
            download_stuck_after: DEFAULT_DOWNLOAD_STUCK_AFTER,
            download_max_attempts: DEFAULT_DOWNLOAD_MAX_ATTEMPTS,
        }
    }
}

impl SyncContext {
    /// Build the per-call download dependency bundle (production retry policy).
    pub fn download_options(&self) -> DownloadOptions {
        DownloadOptions {
            media_root: self.media_root.clone(),
            use_mock_download: self.use_mock_download,
            tracker: self.tracker.clone(),
            retry: RetryPolicy::production(),
        }
    }

    /// Back-compat context: fetches every targeted podcast (no interval gating),
    /// no auto-download/retention. Used by the legacy `sync` entry point (tests)
    /// where only `max_poll_concurrent` + `no_sync_before` matter.
    pub(crate) fn legacy(
        max_poll_concurrent: usize,
        no_sync_before: Option<DateTime<Utc>>,
    ) -> Self {
        Self {
            no_sync_before,
            max_poll_concurrent,
            ..Default::default()
        }
    }
}

/// Outcome of syncing one podcast feed. `skipped` marks the 304-Not-Modified case
/// (feed unchanged), distinct from a successful poll that simply found nothing new.
pub(crate) struct PodcastSyncOutcome {
    pub new: usize,
    pub updated: usize,
    pub errors: usize,
    pub skipped: bool,
    /// Ids of episodes inserted this run (for the auto-download pass).
    pub new_ids: Vec<i32>,
}

/// Episode data extracted from an RSS feed. Similar to `EpisodeData` but without
/// DB-specific fields (id, podcast_id, downloaded_at, content_file_path,
/// art_file_path, download_status).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RemoteEpisodeData {
    pub title: String,
    pub description: Option<String>,
    pub content_url: String,
    pub art_url: Option<String>,
    pub published_at: Option<DateTime<Utc>>,
    /// Episode length in whole seconds, parsed from `<itunes:duration>`.
    pub duration_secs: Option<i32>,
    pub guid: Option<String>,
    /// Inline `psc:chapters` (Podlove Simple Chapters), already parsed from the
    /// feed item — free, no network. Empty when the item carries none.
    #[serde(default)]
    pub chapters: Vec<RemoteChapter>,
    /// A `podcast:chapters` external JSON URL (Podcasting 2.0). Fetched
    /// best-effort during ingest only when there are no inline chapters.
    #[serde(default)]
    pub chapters_url: Option<String>,
}

/// One parsed chapter marker, pre-persistence. Title + start offset only; the
/// richer Podcasting-2.0 fields (img, url, toc, endTime) are intentionally dropped.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RemoteChapter {
    pub title: String,
    pub starts_at_secs: i32,
}

/// Channel metadata and episodes parsed from an RSS document.
#[derive(Debug, Clone)]
pub struct RemoteFeedData {
    /// Channel-level artwork (`<itunes:image href>` first, then RSS
    /// `<image><url>`). Sampling the subscribed library showed 98/98 feeds
    /// carry one — this is the primary podcast-art source; episode-level
    /// images exist in barely half the items.
    pub art_url: Option<String>,
    /// Channel `<title>` — heals placeholder podcast titles (subscribe-by-URL).
    pub channel_title: Option<String>,
    pub channel_description: Option<String>,
    pub episodes: Vec<RemoteEpisodeData>,
}
