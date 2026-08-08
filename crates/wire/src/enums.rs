#[cfg(feature = "db")]
use sea_orm::prelude::StringLen;
#[cfg(feature = "db")]
use sea_orm::{DeriveActiveEnum, EnumIter};
use serde::{Deserialize, Serialize};
use typeshare::typeshare;

#[cfg_attr(feature = "db", derive(EnumIter, DeriveActiveEnum))]
#[derive(Default, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "db",
    sea_orm(rs_type = "String", db_type = "String(StringLen::N(32))")
)]
#[typeshare]
pub enum DownloadStatus {
    #[default]
    #[serde(rename = "NOT_DOWNLOADED")]
    #[cfg_attr(feature = "db", sea_orm(string_value = "NOT_DOWNLOADED"))]
    NotDownloaded,
    #[serde(rename = "DOWNLOADING")]
    #[cfg_attr(feature = "db", sea_orm(string_value = "DOWNLOADING"))]
    Downloading,
    #[serde(rename = "DOWNLOADED")]
    #[cfg_attr(feature = "db", sea_orm(string_value = "DOWNLOADED"))]
    Downloaded,
    #[serde(rename = "DOWNLOAD_ERROR")]
    #[cfg_attr(feature = "db", sea_orm(string_value = "DOWNLOAD_ERROR"))]
    DownloadError,
    /// Auto-retry budget exhausted (`download_attempts >= max`); terminal, never
    /// auto-retried. A manual `POST /download` can still force a fresh attempt.
    #[serde(rename = "DOWNLOAD_BROKEN")]
    #[cfg_attr(feature = "db", sea_orm(string_value = "DOWNLOAD_BROKEN"))]
    DownloadBroken,
    /// Origin returned 403; terminal, never auto-retried.
    #[serde(rename = "DOWNLOAD_UNAUTHORIZED")]
    #[cfg_attr(feature = "db", sea_orm(string_value = "DOWNLOAD_UNAUTHORIZED"))]
    DownloadUnauthorized,
    /// Origin returned 404; terminal, never auto-retried.
    #[serde(rename = "DOWNLOAD_REMOTE_NOT_FOUND")]
    #[cfg_attr(feature = "db", sea_orm(string_value = "DOWNLOAD_REMOTE_NOT_FOUND"))]
    DownloadRemoteNotFound,
}

impl DownloadStatus {
    pub fn from_string(s: &str) -> Self {
        match s {
            "NOT_DOWNLOADED" => Self::NotDownloaded,
            "DOWNLOADING" => Self::Downloading,
            "DOWNLOADED" => Self::Downloaded,
            "DOWNLOAD_ERROR" => Self::DownloadError,
            "DOWNLOAD_BROKEN" => Self::DownloadBroken,
            "DOWNLOAD_UNAUTHORIZED" => Self::DownloadUnauthorized,
            "DOWNLOAD_REMOTE_NOT_FOUND" => Self::DownloadRemoteNotFound,
            _ => Self::default(),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::NotDownloaded => "NOT_DOWNLOADED",
            Self::Downloading => "DOWNLOADING",
            Self::Downloaded => "DOWNLOADED",
            Self::DownloadError => "DOWNLOAD_ERROR",
            Self::DownloadBroken => "DOWNLOAD_BROKEN",
            Self::DownloadUnauthorized => "DOWNLOAD_UNAUTHORIZED",
            Self::DownloadRemoteNotFound => "DOWNLOAD_REMOTE_NOT_FOUND",
        }
    }
}

impl std::fmt::Display for DownloadStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Per-episode listen state for the current user. Set as a side-effect of the
/// playback upsert handler (cursor vs duration vs the completion threshold).
/// Stored on `episode` (single-user-per-deployment), like `download_status`.
#[cfg_attr(feature = "db", derive(EnumIter, DeriveActiveEnum))]
#[derive(Default, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "db",
    sea_orm(rs_type = "String", db_type = "String(StringLen::N(16))")
)]
#[typeshare]
pub enum PlaybackStatus {
    /// Never started (no playback, or cursor at 0).
    #[default]
    #[serde(rename = "UNPLAYED")]
    #[cfg_attr(feature = "db", sea_orm(string_value = "UNPLAYED"))]
    Unplayed,
    /// Started but not within the completion threshold.
    #[serde(rename = "PLAYED")]
    #[cfg_attr(feature = "db", sea_orm(string_value = "PLAYED"))]
    Played,
    /// Listened into the last `episode_playback_complete_percentage`%.
    #[serde(rename = "FINISHED")]
    #[cfg_attr(feature = "db", sea_orm(string_value = "FINISHED"))]
    Finished,
}

impl PlaybackStatus {
    pub fn from_string(s: &str) -> Self {
        match s {
            "UNPLAYED" => Self::Unplayed,
            "PLAYED" => Self::Played,
            "FINISHED" => Self::Finished,
            _ => Self::default(),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Unplayed => "UNPLAYED",
            Self::Played => "PLAYED",
            Self::Finished => "FINISHED",
        }
    }
}

impl std::fmt::Display for PlaybackStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}
