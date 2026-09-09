//! Response types for the admin server-errors endpoint (`GET /admin/server-errors`): the persisted failure
//! histories for podcast RSS syncs and episode media downloads. The two row shapes are deliberately parallel
//! (id + FK + optional title snapshot + reason + timestamp) — they differ only in what they point at. New error
//! kinds should follow the same shape.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::ResponsableData;
use typeshare::typeshare;

#[typeshare]
/// One podcast RSS sync failure (fetch / body read / parse), newest first in the
/// response. `podcast_title` is resolved at read time (`None` if the podcast
/// vanished between the error and the read — FK cascade makes that a tiny race
/// window, not a stale-history case).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PodcastSyncErrorData {
    pub id: i32,
    pub podcast_id: i32,
    pub podcast_title: Option<String>,
    pub reason: String,
    pub created_at: DateTime<Utc>,
}

#[typeshare]
/// One episode media-download failure, newest first in the response.
/// `episode_title`/`podcast_id` are resolved at read time (same race note as
/// [`PodcastSyncErrorData::podcast_title`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EpisodeDownloadErrorData {
    pub id: i32,
    pub episode_id: i32,
    pub episode_title: Option<String>,
    pub podcast_id: Option<i32>,
    pub reason: String,
    pub created_at: DateTime<Utc>,
}

#[typeshare]
/// Both error histories in one read — the Server Errors page shows them side by
/// side. Each list is newest-first and capped server-side.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerErrorsData {
    pub rss_sync: Vec<PodcastSyncErrorData>,
    pub episode_downloads: Vec<EpisodeDownloadErrorData>,
}

impl ResponsableData for ServerErrorsData {}
