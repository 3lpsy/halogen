use chrono::{DateTime, Utc};

use serde::{Deserialize, Serialize};
use validator::Validate;

use super::{RequestData, RequestableParams, ResponsableData, ResponseData};
use typeshare::typeshare;

// Responses
#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PodcastConfigData {
    pub id: i32,
    pub poll_interval_seconds: Option<u32>,
    pub max_episodes: Option<u32>,
    pub max_concurrent_downloads: Option<u32>,
    pub auto_download_enabled: Option<bool>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// Requests
#[derive(Default, Clone, Debug, Validate, Serialize, Deserialize)]
pub struct PodcastConfigShowParams {
    #[validate(range(min = 1, message = "ID must be a valid integer"))]
    pub id: Option<i32>,
}

#[derive(Default, Debug, Validate, Serialize, Deserialize)]
pub struct PodcastConfigDeleteParams {
    #[validate(range(min = 1, message = "ID must be a valid integer"))]
    pub id: i32,
}

#[typeshare]
#[derive(Debug, Validate, Serialize, Deserialize, Default, Clone)]
pub struct PodcastConfigStoreData {
    #[validate(range(
        min = 0,
        max = 86400,
        message = "Poll interval must be between 0 and 86400 seconds"
    ))]
    pub poll_interval_seconds: Option<u32>,
    #[validate(range(
        min = 1,
        max = 10000,
        message = "Max episodes must be between 1 and 10000"
    ))]
    pub max_episodes: Option<u32>,
    #[validate(range(
        min = 1,
        max = 100,
        message = "Max concurrent downloads must be between 1 and 100"
    ))]
    pub max_concurrent_downloads: Option<u32>,
    #[serde(default)]
    pub auto_download_enabled: Option<bool>,
}

#[typeshare]
#[derive(Debug, Validate, Serialize, Deserialize, Default, Clone, PartialEq)]
pub struct PodcastConfigUpdateData {
    #[validate(range(
        min = 0,
        max = 86400,
        message = "Poll interval must be between 0 and 86400 seconds"
    ))]
    pub poll_interval_seconds: Option<u32>,
    #[validate(range(
        min = 1,
        max = 10000,
        message = "Max episodes must be between 1 and 10000"
    ))]
    pub max_episodes: Option<u32>,
    #[validate(range(
        min = 1,
        max = 100,
        message = "Max concurrent downloads must be between 1 and 100"
    ))]
    pub max_concurrent_downloads: Option<u32>,
    #[serde(default)]
    pub auto_download_enabled: Option<bool>,
}

impl PodcastConfigUpdateData {
    /// Merge this update onto an existing config: a `Some` field is applied, a `None`
    /// field leaves the target unchanged. The single source of truth for the update's
    /// field semantics — the client's optimistic updater
    /// (`SyncService::update_podcast_config_locally`) applies it so an offline edit
    /// mirrors exactly what the server will persist.
    pub fn apply_to(&self, cfg: &mut PodcastConfigData) {
        if self.poll_interval_seconds.is_some() {
            cfg.poll_interval_seconds = self.poll_interval_seconds;
        }
        if self.max_episodes.is_some() {
            cfg.max_episodes = self.max_episodes;
        }
        if self.max_concurrent_downloads.is_some() {
            cfg.max_concurrent_downloads = self.max_concurrent_downloads;
        }
        if self.auto_download_enabled.is_some() {
            cfg.auto_download_enabled = self.auto_download_enabled;
        }
    }
}

// impls
impl From<PodcastConfigData> for PodcastConfigStoreData {
    fn from(data: PodcastConfigData) -> Self {
        PodcastConfigStoreData {
            poll_interval_seconds: data.poll_interval_seconds,
            max_episodes: data.max_episodes,
            max_concurrent_downloads: data.max_concurrent_downloads,
            auto_download_enabled: data.auto_download_enabled,
        }
    }
}

impl From<PodcastConfigData> for PodcastConfigUpdateData {
    fn from(data: PodcastConfigData) -> Self {
        PodcastConfigUpdateData {
            poll_interval_seconds: data.poll_interval_seconds,
            max_episodes: data.max_episodes,
            max_concurrent_downloads: data.max_concurrent_downloads,
            auto_download_enabled: data.auto_download_enabled,
        }
    }
}

impl<P: RequestableParams> From<PodcastConfigStoreData> for RequestData<PodcastConfigStoreData, P> {
    fn from(data: PodcastConfigStoreData) -> Self {
        RequestData::from_data(data)
    }
}

impl From<PodcastConfigData> for ResponseData<PodcastConfigData> {
    fn from(data: PodcastConfigData) -> Self {
        ResponseData {
            data: Some(data),
            errors: None,
            paginator: None,
        }
    }
}

impl ResponsableData for PodcastConfigData {}
