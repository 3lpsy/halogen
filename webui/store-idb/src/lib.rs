//! Wasm-only IndexedDB helpers: [`ceremony`] handles transactions/errors, [`json`] stores typed JSON values, and
//! [`schema`] manages versioned stores. New stores use [`open`], which resets caches but preserves durable data; legacy
//! media/logging stores use ceremony directly.

#[cfg(target_arch = "wasm32")]
mod ceremony;
#[cfg(target_arch = "wasm32")]
mod guard;
#[cfg(target_arch = "wasm32")]
mod json;
#[cfg(target_arch = "wasm32")]
mod schema;

#[cfg(target_arch = "wasm32")]
pub use ceremony::{idb_err, one_store_tx};
// Drop-safe await wrappers (see `guard`): `commit_tx`/`tx_done` flush a transaction
// and `guarded` wraps any request so a cancelled IndexedDB op can't fire a freed
// wasm-bindgen closure (the idb 0.6.5 dangling-handler panic).
#[cfg(target_arch = "wasm32")]
pub use guard::{commit_tx, guarded, tx_done};
#[cfg(target_arch = "wasm32")]
pub use json::{
    clear_store, delete, get_all_episodes, get_all_json, get_all_with_keys, get_json, num_key,
    page_by_index, put_episode, put_json, scan_index_eq, scan_index_eq_ids, str_key,
};
#[cfg(target_arch = "wasm32")]
pub use schema::{IndexSpec, Opened, Schema, StoreKind, StoreSpec, delete_db, open, open_current};
