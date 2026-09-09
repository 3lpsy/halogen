//! Persist list-view state separately from auth config so sort/filter changes cannot trigger global config rerenders or
//! resyncs. ConfigProvider's ListViews signal owns runtime state.

use serde::{Deserialize, Serialize};

use super::store::NamespacedStore;

/// Persist sort/filter values as stable tokens; empty sort_field means unset. Keep search only in URL state for
/// reload/back/shared links, clearing it on fresh navigation so old searches do not silently hide rows.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct StoredListView {
    #[serde(default)]
    pub sort_field: String,
    #[serde(default)]
    pub sort_dir: String,
    #[serde(default)]
    pub filters: Vec<String>,
}

/// Remembered per-list view state (sort / filter chips), keyed by list *type*
/// (`"latest"`, `"podcast"`, `"queue"`, …). Lets a list restore its last view on
/// fresh navigation and across restarts; the live URL query is the
/// within-session / shareable mirror of the same state.
pub type ListViews = std::collections::HashMap<String, StoredListView>;

/// Persistent storage backend for [`ListViews`], namespaced per active user (set by `AccountsProvider`), like
/// [`ClientConfigStore`](super::ClientConfigStore). Web: IndexedDB `halogen.config.u{id}`, record `kv["list_views"]`.
/// Native: JSON file at `{config_dir}/halogen/u{id}/list_views.json`.
pub struct ListViewStore;

/// The shared namespaced-store backend for [`ListViews`] (key suffix `list_views`
/// / file `list_views.json`).
const VIEW_STORE: NamespacedStore<ListViews> =
    NamespacedStore::new("list_views", "list_views.json");

impl ListViewStore {
    /// Load the active user's list-view map, empty if nothing is stored.
    pub async fn load() -> ListViews {
        VIEW_STORE.load().await
    }

    /// Persist the active user's list-view map.
    pub async fn save(views: &ListViews) {
        VIEW_STORE.save(views).await;
    }

    /// Remove the active user's persisted list-view state (part of a local-data wipe).
    pub async fn clear() {
        VIEW_STORE.clear().await;
    }

    /// Remove a *specific* account's persisted list-view state. Used when signing
    /// out or wiping an account that may not be the active namespace (the ambient
    /// `clear` can only reach the active user, orphaning everyone else's state).
    pub async fn clear_for(key: crate::AccountKey) {
        VIEW_STORE.clear_for(key).await;
    }
}
