//! API <-> DB boundary DTOs and supporting types. Compiles with or without the
//! `db` feature (WASM-safe).
//!
//! # Core Concepts
//!
//! - **Request/Response**: `RequestData<T, P>` in, `ResponseData<T>` out.
//! - **Validation**: every `*Data`/`*Params` derives `validator::Validate`.
//! - **Params**: query params via `*Params` (show/list/delete).
//! - **Store/Update**: `*StoreData` (create) / `*UpdateData` (patch) feed the ORM.
//! - **Includes**: eager-loaded relations via `HasIncludes` / `Includable`.
//!
//! # Usage Examples
//!
//! ## Creating a Request
//! ```rust
//! use halogen_wire::{PodcastStoreData, RequestData};
//!
//! let store_data = PodcastStoreData {
//!     title: "My Podcast".to_string(),
//!     feed_url: "https://example.com/feed.xml".to_string(),
//!     description: Some("A great podcast".to_string()),
//!     art_url: None,
//!     author: None,
//!     podcast_config_id: None,
//! };
//!
//! let _request: RequestData<PodcastStoreData, halogen_wire::DefaultParamsType> = RequestData::from_data(store_data);
//! ```
//!
//! ## Handling a Response
//! ```rust
//! use halogen_wire::{PodcastData, ResponseData};
//!
//! let podcast_data = PodcastData {
//!     id: 1,
//!     title: "My Podcast".to_string(),
//!     description: "A great podcast".to_string(),
//!     feed_url: "https://example.com/feed.xml".to_string(),
//!     art_url: None,
//!     art_file_path: None,
//!     author: None,
//!     etag: None,
//!     last_modified: None,
//!     polled_at: None,
//!     podcast_config_id: None,
//!     podcast_config: None,
//!     created_at: chrono::Utc::now(),
//!     updated_at: chrono::Utc::now(),
//!     episode_count: None,
//!     feed_url_redirects: None,
//! };
//!
//! let _response = ResponseData::from_data(podcast_data);
//! ```
//!
//! ## Working with Pagination
//! ```rust
//! use halogen_wire::Pagination;
//!
//! let pagination = Pagination::default();
//! assert_eq!(pagination.page, 0);
//! assert_eq!(pagination.size, 10);
//! ```
//!
//! ## Using Includes
//! ```rust
//! use halogen_wire::{PodcastInclude, HasIncludes};
//!
//! let mut includes = Vec::new();
//! includes.push(PodcastInclude::PodcastConfig);
//! ```

/// The meta layer (request/response envelopes, pagination, includes, DB errors)
/// lives in its own crate; re-exported as `meta` so existing `crate::meta::*`
/// and `halogen_wire::meta::*` paths keep resolving.
pub use halogen_wire_meta as meta;
pub use halogen_wire_meta::field_ref;

pub mod auth;
pub mod config;
pub mod db_transfer;
pub mod discover;
pub mod download_progress;
pub mod enums;
pub mod episode;
pub mod episode_chapter;
pub mod episode_playlist;
pub mod includes;
pub mod opml;
pub mod playback;
pub mod playlist;
pub mod podcast;
pub mod podcast_auto_playlist;
pub mod podcast_config;
pub mod polling;
pub mod server_errors;
pub mod server_logs;
pub mod user;
pub mod ws;
pub use auth::{LoginData, StatusData, TokenData, VersionData};
pub use config::{ConfigData, ConfigOverridesData};
pub use discover::{
    DiscoverProvider, DiscoverProviderError, DiscoverProviderInfo, DiscoverProvidersData,
    DiscoverResultItem, DiscoverSearchData, DiscoverSearchParams,
};
pub use download_progress::DownloadProgressData;
pub use enums::{DownloadStatus, PlaybackStatus};
pub use episode::{
    EpisodeBulkActionData, EpisodeData, EpisodeDeleteParams, EpisodeShowParams, EpisodeStoreData,
    EpisodeUpdateData,
};
pub use episode_chapter::EpisodeChapterData;
pub use episode_playlist::{
    EpisodePlaylistBulkData, EpisodePlaylistData, EpisodePlaylistDeleteParams,
    EpisodePlaylistMoveData, EpisodePlaylistStoreData, EpisodePlaylistStoreParams,
};
pub use includes::{EpisodeInclude, PodcastInclude};
#[cfg(feature = "db")]
pub use meta::db::DbValidationErrors;
pub use meta::get::DefaultGetParams;
pub use meta::includes::{HasIncludes, Includable, NoInclude};
pub use meta::list::{DefaultListParams, FilterParams};
pub use meta::order::{HasOrder, Order, OrderDirection, cmp_opt};
pub use meta::pagination::{HasPagination, Pagination, Paginator};
pub use meta::request::{
    DefaultDataType, DefaultParamsType, RequestData, RequestableData, RequestableParams,
};
pub use meta::response::{
    Page, ResponsableData, ResponseData, SerializableValidationErrors, ValidationErrorField,
};
pub use opml::{OpmlExportData, OpmlImportData, OpmlImportResultData};
pub use playback::{
    PlaybackData, PlaybackDeleteParams, PlaybackListParams, PlaybackShowParams, PlaybackStoreData,
};
pub use playlist::{
    DefaultPlaylistData, PlaylistData, PlaylistDeleteParams, PlaylistInclude, PlaylistListParams,
    PlaylistMoveData, PlaylistReorderData, PlaylistReorderField, PlaylistShowParams,
    PlaylistStoreData, PlaylistUpdateData,
};
pub use podcast::{
    PodcastData, PodcastDeleteParams, PodcastShowParams, PodcastStoreData, PodcastUpdateData,
};
pub use podcast_auto_playlist::{PodcastAutoPlaylistData, PodcastAutoPlaylistSetData};
pub use podcast_config::{
    PodcastConfigData, PodcastConfigDeleteParams, PodcastConfigShowParams, PodcastConfigStoreData,
    PodcastConfigUpdateData,
};
pub use polling::{
    PodcastPollOutcome, PodcastPollResultData, PollJobData, PollJobStartData, PollJobStatus,
    PollJobTrigger, PollingOperationData, PollingStatusData, outcome_for,
};
pub use server_errors::{EpisodeDownloadErrorData, PodcastSyncErrorData, ServerErrorsData};
pub use server_logs::ServerLogsData;
pub use user::{
    PasswordChangeData, PasswordUpdateData, UserData, UserDeleteParams, UserListParams,
    UserShowParams, UserStoreData, UserUpdateData,
};
pub use validator::{Validate, ValidationError, ValidationErrors};
pub use ws::WsTicketData;

#[cfg(test)]
mod tests;
pub use db_transfer::DbImportSummaryData;
