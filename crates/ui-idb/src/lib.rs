//! `halogen-ui-idb` — the shared IndexedDB foundation for the web UI stores.
//!
//! Three layers, all `#[cfg(target_arch = "wasm32")]` (native is an empty crate):
//!
//! - [`ceremony`] — the per-call boilerplate every IndexedDB op repeats:
//!   `idb_err` (tag a fallible step), `one_store_tx` (open a single-store
//!   transaction), `commit_tx` (commit + await).
//! - [`json`] — typed record helpers that serialize each value as a JSON-string
//!   `JsValue` (so `serde_wasm_bindgen` stays out of the dep graph) keyed by an
//!   explicit out-of-line key: `put_json` / `get_json` / `get_all_json` /
//!   `get_all_with_keys` / `delete` / `clear_store` + the `num_key`/`str_key`
//!   constructors.
//! - [`schema`] — the **versioned-open + migration framework**: declare a
//!   [`Schema`] (db name + version + [`StoreSpec`]s), call [`open`], and on a
//!   version bump the framework recreates `Cache` stores (re-fetchable content)
//!   and preserves `Durable` ones (outbox / config / accounts). [`delete_db`]
//!   backs sign-out + the cache-control failsafe.
//!
//! Existing web stores (`halogen-ui-svc-media`, `halogen-ui-logging`) predate this
//! crate and use the ceremony helpers directly; new stores open through [`open`].

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
