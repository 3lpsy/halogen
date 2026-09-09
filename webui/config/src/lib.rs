//! Local client config and persistence, action wrappers, playback/download/font preferences, swipe actions, and
//! navigation. List-view state has a separate store to avoid writing auth-bearing config on list changes.

mod config;
pub mod config_actions;
mod list_view;
mod nav;
mod prefs;
mod server_kind;
mod store;
mod swipe;
#[cfg(target_arch = "wasm32")]
mod web;

#[cfg(not(target_arch = "wasm32"))]
pub use config::set_embedded_url_resolver;
pub use config::{ClientConfig, ClientConfigStore, DeviceLogConfig, api_client_from};
pub use server_kind::{AccountKey, ServerKind, server_hash};
// The native file-path helper is reached by the store submodule via `super::`
// (web persists through `crate::web`/IndexedDB and needs no path helper).
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use config::ns_path;
pub use list_view::{ListViewStore, ListViews, StoredListView};
pub use nav::{BuiltinNav, NavConfig, NavKey};
pub use prefs::{
    DOWNLOAD_PARALLELISMS, DownloadChunkSize, DownloadPrefs, FontSize, PLAYBACK_RATES,
    PlaybackPreference, PlaybackPrefs, SLEEP_DURATIONS, SLEEP_INCREMENTS, SelectEnum,
};
pub use swipe::{SwipePage, SwipePrefs};
