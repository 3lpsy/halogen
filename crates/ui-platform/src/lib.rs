//! `halogen-ui-platform` — cross-target platform glue for the UI crate graph.
//!
//! - `time::sleep_ms` / `time::now_ms` — one async sleep + a wall-clock epoch-ms
//!   reading over tokio/`SystemTime` (native) / gloo + `Date.now()` (wasm).
//! - the `kv_load_opt!` / `kv_save!` / `kv_delete!` macros (`#[macro_export]`ed to
//!   the crate root) — JSON file persistence (native only; the web stores are on
//!   IndexedDB via `halogen-ui-idb`).
//! - `paths` (native only) — the per-OS app data/config roots every native store
//!   resolves against (XDG/`Library`, flatpak-safe).
//! - `namespace` — the ambient active-user storage namespace.
//! - `store_keys` — the shared web IndexedDB database/store names.
//!
//! Tiny and dependency-light; a foundation the store/config/media/logging layers
//! build on. (IndexedDB ceremony lives in the sibling `halogen-ui-idb` crate.)

mod kv;
pub mod namespace;
#[cfg(not(target_arch = "wasm32"))]
pub mod paths;
pub mod store_keys;
pub mod time;
#[cfg(target_arch = "wasm32")]
pub mod weblock;
