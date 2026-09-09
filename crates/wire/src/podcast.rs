use chrono::{DateTime, Utc};

use serde::{Deserialize, Serialize};
use validator::Validate;

use super::{PodcastInclude, RequestData, RequestableParams, ResponsableData, ResponseData};
use typeshare::typeshare;

// Responses
#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PodcastData {
    pub id: i32,
    pub title: String,
    pub description: String,
    pub feed_url: String,
    pub art_url: Option<String>,
    pub author: Option<String>,
    pub polled_at: Option<DateTime<Utc>>,
    pub podcast_config_id: Option<i32>,
    pub art_file_path: Option<String>,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub podcast_config: Option<super::PodcastConfigData>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Number of episodes this podcast has, when the server populates it (list /
    /// get). `None` when not computed (e.g. a client-built value). Lets the UI show
    /// per-podcast counts without holding every episode in memory.
    #[serde(default)]
    #[typeshare(serialized_as = "U53")]
    pub episode_count: Option<u64>,
    /// Redirect hop chain seen on the last poll, as a comma-joined CSV starting
    /// with `feed_url`. A direct feed equals `feed_url` exactly, so the UI flags
    /// redirects with `feed_url_redirects != feed_url`. `None` until first poll /
    /// when not populated (server-only; never accepted from the client).
    #[serde(default)]
    pub feed_url_redirects: Option<String>,
}

// Requests
#[derive(Default, Clone, Debug, Validate, Serialize, Deserialize)]
pub struct PodcastShowParams {
    #[validate(range(min = 1, message = "ID must be a valid integer"))]
    pub id: Option<i32>,
    #[validate(length(max = 256, message = "Title must be at most 256 characters long"))]
    pub title: Option<String>,
    #[validate(length(max = 10, message = "Max 10 includes allowed"))]
    pub includes: Option<Vec<PodcastInclude>>,
}

impl super::HasIncludes<PodcastInclude> for PodcastShowParams {
    fn includes(&mut self) -> &mut Option<Vec<PodcastInclude>> {
        &mut self.includes
    }
}

#[derive(Default, Debug, Validate, Serialize, Deserialize)]
pub struct PodcastDeleteParams {
    #[validate(range(min = 1, message = "ID must be a valid integer"))]
    pub id: i32,
}

#[typeshare]
#[derive(Debug, Validate, Serialize, Deserialize, Default, Clone)]
pub struct PodcastStoreData {
    #[validate(length(
        min = 1,
        max = 256,
        message = "Title must be between 1 and 256 characters long"
    ))]
    pub title: String,
    #[validate(length(
        max = 65536,
        message = "Description must be at most 65536 characters long"
    ))]
    pub description: Option<String>,
    #[validate(url(message = "Feed URL must be a valid URL"))]
    pub feed_url: String,
    #[validate(url(message = "Art URL must be a valid URL"))]
    pub art_url: Option<String>,
    #[validate(length(max = 256, message = "Author must be at most 256 characters long"))]
    pub author: Option<String>,
    pub podcast_config_id: Option<i32>,
}

#[typeshare]
#[derive(Debug, Validate, Serialize, Deserialize, Default, Clone)]
pub struct PodcastUpdateData {
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
    #[validate(url(message = "Feed URL must be a valid URL"))]
    pub feed_url: Option<String>,
    // `art_url` and `author` are intentionally NOT editable here: both are feed-derived (art flows through the
    // server art cache; author comes from the RSS channel), no client form sets them, and as nullable columns
    // an `Option<String>` field could never express "clear to NULL" anyway. Omitted rather than carried as
    // dead, half-working input.
}

// impls
impl From<PodcastData> for PodcastStoreData {
    fn from(data: PodcastData) -> Self {
        PodcastStoreData {
            title: data.title,
            description: data.description.into(),
            feed_url: data.feed_url,
            art_url: data.art_url.clone(),
            author: data.author.clone(),
            podcast_config_id: data.podcast_config_id,
        }
    }
}

impl From<PodcastData> for PodcastUpdateData {
    fn from(data: PodcastData) -> Self {
        PodcastUpdateData {
            title: Some(data.title),
            description: Some(data.description),
            feed_url: Some(data.feed_url),
        }
    }
}

impl<P: RequestableParams> From<PodcastStoreData> for RequestData<PodcastStoreData, P> {
    fn from(data: PodcastStoreData) -> Self {
        RequestData::from_data(data)
    }
}

impl From<PodcastData> for ResponseData<PodcastData> {
    fn from(data: PodcastData) -> Self {
        ResponseData {
            data: Some(data),
            errors: None,
            paginator: None,
        }
    }
}

impl ResponsableData for PodcastData {}

// `.validate()` is sync and DB-free, so these run without the `db` feature.
