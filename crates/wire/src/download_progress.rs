use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::ResponsableData;
use typeshare::typeshare;

#[typeshare]
/// A snapshot of one in-flight episode download, served from the server's
/// in-memory `DownloadTracker`. Exists only while a download is running; once it
/// reaches a terminal state the entry is dropped and the API 404s (the durable
/// outcome lives on `episode.download_status`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DownloadProgressData {
    pub episode_id: i32,
    #[typeshare(serialized_as = "U53")]
    pub bytes_downloaded: u64,
    /// Total size from the origin's `Content-Length`, when advertised. `None` for
    /// chunked/unknown-length responses.
    #[typeshare(serialized_as = "Option<U53>")]
    pub total_bytes: Option<u64>,
    /// `bytes_downloaded / total_bytes` in `0.0..=1.0`; `None` when total unknown.
    pub percent: Option<f32>,
    pub started_at: DateTime<Utc>,
}

impl ResponsableData for DownloadProgressData {}
