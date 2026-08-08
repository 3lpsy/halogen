//! Local-only client configuration. Re-export shell.
//!
//! - [`config`] — the `ClientConfig` blob + its per-user `ClientConfigStore`,
//!   `DeviceLogConfig`, `api_client_from`, and the namespaced key/path helpers.
//! - [`config_actions`] — the mutate-and-persist action wrappers.
//! - [`prefs`] — font size, playback + download preference value types.
//! - [`swipe`] — per-page episode swipe-action preferences.
//! - [`nav`] — nav ordering / visibility config.
//! - [`list_view`] — remembered per-list view state + its store (held *outside*
//!   `ClientConfig`; see that module).

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
