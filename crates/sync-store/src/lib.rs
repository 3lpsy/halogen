//! Metadata persistence contract and native query adapter.
pub mod outbox {
    pub use halogen_sync_enrich::OutboxOp;
}
pub use halogen_sync_enrich::{
    EpisodeOrder, EpisodeQuery, EpisodeQueryFilter, JournalEntry, OutboxOp, StoreChanges,
    StoredEntry, filter_count, filter_sort_paginate,
};
mod store;
pub use store::{LocalStore, StoreHandle};
#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(not(target_arch = "wasm32"))]
pub use native::NativeLocalStore;
