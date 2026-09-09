use chrono::{DateTime, Utc};

use serde::{Deserialize, Serialize};
use typeshare::typeshare;
use validator::Validate;

use super::{
    DownloadStatus, EpisodeInclude, PlaybackStatus, RequestData, RequestableParams,
    ResponsableData, ResponseData,
};

// Responses
#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EpisodeData {
    pub id: i32,
    pub podcast_id: i32,
    pub title: String,
    pub description: Option<String>,
    pub content_url: String,
    /// Stable RSS `<guid>`, preferred over `content_url` for episode identity.
    /// `#[serde(default)]` keeps older payloads/caches (written before this
    /// field) readable.
    #[serde(default)]
    pub guid: Option<String>,
    pub art_url: Option<String>,
    pub published_at: Option<DateTime<Utc>>,
    pub downloaded_at: Option<DateTime<Utc>>,
    pub content_file_path: Option<String>,
    /// Size in bytes of the server-side download (when present). `#[serde(default)]`
    /// keeps older payloads/caches (written before this field) readable.
    #[serde(default)]
    #[typeshare(serialized_as = "U53")]
    pub download_size: Option<u64>,
    pub art_file_path: Option<String>,
    pub download_status: DownloadStatus,
    /// When the current/last download attempt started. `#[serde(default)]` keeps
    /// older payloads/caches (written before this field) readable.
    #[serde(default)]
    pub download_started_at: Option<DateTime<Utc>>,
    /// Number of download attempts so far. `#[serde(default)]` keeps older
    /// payloads/caches (written before this field) readable.
    #[serde(default)]
    pub download_attempts: i32,
    /// Per-user listen state (Unplayed/Played/Finished). `#[serde(default)]` keeps
    /// older payloads/caches (written before this column) readable.
    #[serde(default)]
    pub playback_status: PlaybackStatus,
    pub duration_secs: Option<i32>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Optional joined parent podcast (populated by includes).
    pub podcast: Option<super::PodcastData>,
    /// Optional joined playback position for the current user. The server leaves
    /// this `None`; clients fill it from their playback set to drive progress /
    /// unplayed state. `#[serde(default)]` keeps older payloads/caches readable.
    #[serde(default)]
    pub playback: Option<super::PlaybackData>,
    /// Optional ordered chapter markers (populated only by the
    /// `EpisodeInclude::Chapters` embed). The server leaves this `None` unless
    /// requested. `#[serde(default)]` keeps older payloads/caches (written before
    /// this field) readable.
    #[serde(default)]
    pub chapters: Option<Vec<super::EpisodeChapterData>>,
}

// Requests
#[derive(Default, Clone, Debug, Validate, Serialize, Deserialize)]
pub struct EpisodeShowParams {
    #[validate(range(min = 1, message = "ID must be a valid integer"))]
    pub id: Option<i32>,
    #[validate(range(min = 1, message = "Podcast ID must be a valid integer"))]
    pub podcast_id: Option<i32>,
    #[validate(length(max = 10, message = "Max 10 includes allowed"))]
    pub includes: Option<Vec<EpisodeInclude>>,
}

impl super::HasIncludes<EpisodeInclude> for EpisodeShowParams {
    fn includes(&mut self) -> &mut Option<Vec<EpisodeInclude>> {
        &mut self.includes
    }
}

#[derive(Default, Debug, Validate, Serialize, Deserialize)]
pub struct EpisodeDeleteParams {
    #[validate(range(min = 1, message = "ID must be a valid integer"))]
    pub id: i32,
}

#[derive(Debug, Validate, Serialize, Deserialize, Default, Clone)]
pub struct EpisodeStoreData {
    #[validate(range(min = 1, message = "Podcast ID must be a valid integer"))]
    pub podcast_id: i32,
    #[validate(length(
        min = 1,
        max = 256,
        message = "Title must be between 1 and 256 characters long"
    ))]
    pub title: String,
    // Required: the `episode.description` column is NOT NULL. Keeping this a plain
    // `String` (not `Option`) means the length validator ALWAYS runs and a missing
    // description can never reach the DB as NULL (which would 500 at insert time).
    #[validate(length(
        max = 65536,
        message = "Description must be at most 65536 characters long"
    ))]
    pub description: String,
    #[validate(url(message = "Content URL must be a valid URL"))]
    pub content_url: String,
    pub guid: Option<String>,
    #[validate(url(message = "Art URL must be a valid URL"))]
    pub art_url: Option<String>,
    pub published_at: Option<DateTime<Utc>>,
    // Reject client-supplied file paths and download status/timestamps. Only server download/art pipelines may set this
    // bookkeeping under media_root; accepting arbitrary paths would expose local files through media endpoints.
    pub duration_secs: Option<i32>,
}

#[derive(Debug, Validate, Serialize, Deserialize, Default, Clone)]
pub struct EpisodeUpdateData {
    #[validate(length(
        min = 1,
        max = 256,
        message = "Title must be between 1 and 256 characters long"
    ))]
    pub title: Option<String>,
    #[validate(length(
        max = 65536,
        message = "Description must be at most 65536 characters long"
    ))]
    pub description: Option<String>,
    #[validate(url(message = "Content URL must be a valid URL"))]
    pub content_url: Option<String>,
    #[validate(url(message = "Art URL must be a valid URL"))]
    pub art_url: Option<String>,
    pub published_at: Option<DateTime<Utc>>,
    // NOTE: server-managed download fields (`downloaded_at`, `content_file_path`,
    // `art_file_path`, `download_status`) are intentionally omitted — see
    // `EpisodeStoreData`. A client update carries only metadata; those columns
    // move solely through the download pipeline.
}

/// Body for the bulk download/remove endpoints (`POST`/`DELETE
/// /episodes/download/bulk`). One request carries many episode ids; the handler
/// filters them to the ones the actor is authorized for, then loops the per-episode
/// service. `min = 1` keeps an empty request a clean 400; `max` bounds the loop.
#[derive(Debug, Validate, Serialize, Deserialize, Default, Clone)]
pub struct EpisodeBulkActionData {
    #[validate(length(
        min = 1,
        max = 500,
        message = "Must include between 1 and 500 episode ids"
    ))]
    pub episode_ids: Vec<i32>,
}

// impls
impl From<EpisodeData> for EpisodeStoreData {
    fn from(data: EpisodeData) -> Self {
        EpisodeStoreData {
            podcast_id: data.podcast_id,
            title: data.title,
            description: data.description.clone().unwrap_or_default(),
            content_url: data.content_url,
            guid: data.guid,
            art_url: data.art_url.clone(),
            published_at: data.published_at,
            duration_secs: data.duration_secs,
        }
    }
}

impl From<EpisodeData> for EpisodeUpdateData {
    fn from(data: EpisodeData) -> Self {
        EpisodeUpdateData {
            title: Some(data.title),
            description: data.description.clone(),
            content_url: Some(data.content_url),
            art_url: data.art_url.clone(),
            published_at: data.published_at,
        }
    }
}

impl<P: RequestableParams> From<EpisodeStoreData> for RequestData<EpisodeStoreData, P> {
    fn from(data: EpisodeStoreData) -> Self {
        RequestData::from_data(data)
    }
}

impl From<EpisodeData> for ResponseData<EpisodeData> {
    fn from(data: EpisodeData) -> Self {
        ResponseData {
            data: Some(data),
            errors: None,
            paginator: None,
        }
    }
}

impl ResponsableData for EpisodeData {}
