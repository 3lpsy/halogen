//! In-memory progress tracker for in-flight episode downloads.
//!
//! Mirrors `halogen_polling::jobs::JobTracker`: a
//! thread-safe map, cheap to clone-share behind an `Arc`. An entry exists only
//! while a download is running — [`DownloadTracker::begin`] inserts/resets it and
//! [`DownloadTracker::finish`] removes it on any terminal state, so the read API
//! 404s once a download is done (the durable outcome lives on
//! `episode.download_status`). State is process memory only; lost on restart.
//!
//! The hot path is the chunk loop, which holds the per-entry `Arc<ProgressEntry>`
//! and does a lock-free `downloaded.fetch_add(..)` per chunk — it never touches
//! the map mutex after `begin`.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::{DateTime, Utc};
use halogen_wire::DownloadProgressData;

/// Per-download progress, shared between the chunk loop (writer) and the read
/// API (reader). `downloaded` is an atomic so the loop never locks the map.
/// Fields are private: the writer accumulates via [`ProgressEntry::add`] and the
/// reader goes through [`DownloadTracker::get`]/[`active`](DownloadTracker::active).
#[derive(Debug)]
pub struct ProgressEntry {
    /// Origin `Content-Length`, when advertised; `None` for chunked responses.
    total: Option<u64>,
    downloaded: AtomicU64,
    started_at: DateTime<Utc>,
}

impl ProgressEntry {
    /// Accumulate `n` freshly-downloaded bytes (the chunk-loop hot path). Lock-free.
    pub fn add(&self, n: u64) {
        self.downloaded.fetch_add(n, Ordering::Relaxed);
    }

    fn snapshot(&self, episode_id: i32) -> DownloadProgressData {
        let downloaded = self.downloaded.load(Ordering::Relaxed);
        let percent = match self.total {
            Some(total) if total > 0 => {
                Some((downloaded as f64 / total as f64).clamp(0.0, 1.0) as f32)
            }
            _ => None,
        };
        DownloadProgressData {
            episode_id,
            bytes_downloaded: downloaded,
            total_bytes: self.total,
            percent,
            started_at: self.started_at,
        }
    }
}

/// Thread-safe map of episode id → in-flight download progress.
#[derive(Debug, Default)]
pub struct DownloadTracker {
    inner: Mutex<HashMap<i32, Arc<ProgressEntry>>>,
}

impl DownloadTracker {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }

    /// Start (or reset) tracking for `episode_id` with the given total size,
    /// returning the entry the chunk loop accumulates into. Called once per
    /// download attempt, so a retried attempt resets the counter to 0.
    pub fn begin(&self, episode_id: i32, total: Option<u64>) -> Arc<ProgressEntry> {
        let entry = Arc::new(ProgressEntry {
            total,
            downloaded: AtomicU64::new(0),
            started_at: Utc::now(),
        });
        self.lock().insert(episode_id, entry.clone());
        entry
    }

    /// Drop the entry for `episode_id` (download reached a terminal state). A
    /// no-op if it was never tracked.
    pub fn finish(&self, episode_id: i32) {
        self.lock().remove(&episode_id);
    }

    /// Snapshot of one in-flight download, if currently tracked.
    pub fn get(&self, episode_id: i32) -> Option<DownloadProgressData> {
        self.lock().get(&episode_id).map(|e| e.snapshot(episode_id))
    }

    /// Snapshot of every in-flight download.
    pub fn active(&self) -> Vec<DownloadProgressData> {
        self.lock().iter().map(|(id, e)| e.snapshot(*id)).collect()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<i32, Arc<ProgressEntry>>> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }
}
