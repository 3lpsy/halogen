//! The typed [`ApiClient`] and its request methods.

use futures_util::StreamExt;
use reqwest::Client;
use serde::de::DeserializeOwned;
use url::Url;

use halogen_wire::{
    ConfigData, ConfigOverridesData, DbImportSummaryData, DefaultPlaylistData,
    DiscoverProvidersData, DiscoverSearchData, DiscoverSearchParams, EpisodeData, EpisodeInclude,
    EpisodePlaylistBulkData, EpisodePlaylistData, EpisodePlaylistMoveData,
    EpisodePlaylistStoreData, EpisodeUpdateData, LoginData, OpmlExportData, OpmlImportData,
    OpmlImportResultData, Page, PasswordChangeData, PlaybackData, PlaybackListParams,
    PlaybackStoreData, PlaylistData, PlaylistInclude, PlaylistStoreData, PlaylistUpdateData,
    PodcastAutoPlaylistData, PodcastAutoPlaylistSetData, PodcastConfigData, PodcastConfigStoreData,
    PodcastConfigUpdateData, PodcastData, PodcastInclude, PodcastStoreData, PodcastUpdateData,
    PollJobData, PollJobStartData, PollingOperationData, PollingStatusData, ResponsableData,
    ServerErrorsData, ServerLogsData, StatusData, TokenData, UserData, UserStoreData,
    UserUpdateData, WsTicketData,
};

use crate::audio::{AudioChunk, AudioStream};
use crate::error::{ApiError, parse_error_body};
use crate::parse::{parse_content_range_start, parse_content_range_total};

/// Typed API client for the Halogen backend. `token` is interior-mutable so the sync worker and auth flow can
/// update it after login/refresh without rebuilding the client. Every authed call attaches `Authorization:
/// Bearer <token>`.
pub struct ApiClient {
    http: Client,
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) local: Option<std::sync::Arc<dyn crate::LocalTransport>>,
    /// Base URL with any trailing `/` stripped, so `format!("{base}{path}")`
    /// (path starts with `/`) never produces a `//` that fails route matching.
    base: String,
    token: std::sync::RwLock<Option<String>>,
}

/// Native HTTP client with a connect timeout so unreachable servers fail promptly. No read/total timeout: audio
/// transfers may be large and `/admin/poll` can wait without response bytes while fetching feeds. WASM uses browser
/// fetch without builder timeouts.
fn build_http() -> Client {
    #[cfg(not(target_arch = "wasm32"))]
    {
        Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .build()
            .unwrap_or_else(|_| Client::new())
    }
    #[cfg(target_arch = "wasm32")]
    {
        Client::new()
    }
}

impl ApiClient {
    /// Create a new `ApiClient` pointing at `base_url`. All HTTP APIs live under `/api/v1` (so they never
    /// shadow the SPA's client-side routes), so the prefix is baked into the base here — every path method
    /// below stays prefix-free.
    pub fn new(base_url: Url) -> Self {
        Self {
            http: build_http(),
            #[cfg(not(target_arch = "wasm32"))]
            local: crate::transport::resolve(&base_url),
            base: format!("{}/api/v1", base_url.as_str().trim_end_matches('/')),
            token: std::sync::RwLock::new(None),
        }
    }

    /// An independent client with the same base URL and a snapshot of the
    /// current token. For detached tasks (e.g. the device-download fetch) that
    /// outlive the borrow of the owner's client. Token updates after this call
    /// don't propagate — fine for short-lived tasks.
    pub fn clone_handle(&self) -> Self {
        Self {
            http: build_http(),
            #[cfg(not(target_arch = "wasm32"))]
            local: self.local.clone(),
            base: self.base.clone(),
            token: std::sync::RwLock::new(self.token()),
        }
    }

    /// Set the auth token (called after login/refresh).
    pub fn set_token(&self, token: Option<String>) {
        *self.token.write().unwrap() = token;
    }

    /// Get the current auth token.
    pub fn token(&self) -> Option<String> {
        self.token.read().unwrap().clone()
    }
}

/// Build a URL with optional query string.
fn build_url(base: &str, path: &str, qs: &str) -> String {
    if qs.is_empty() {
        format!("{}{}", base, path)
    } else {
        format!("{}{}?{}", base, path, qs)
    }
}

mod administration;
mod auth;
mod episode_audio;
mod episode_downloads;
mod library;
mod playbacks;
mod playlist_episodes;
mod playlists;
mod podcast_auto_playlists;
mod podcast_configs;
mod podcasts;
mod requests;
mod responses;
mod sync;
mod users;
