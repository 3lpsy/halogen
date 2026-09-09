//! Dependency-light platform helpers provide cross-target time, native JSON persistence and storage roots,
//! active-account namespaces, and shared browser store names. IndexedDB operations live in the sibling store-idb crate.

mod kv;
pub mod namespace;
#[cfg(not(target_arch = "wasm32"))]
pub mod paths;
pub mod store_keys;
pub mod time;
#[cfg(target_arch = "wasm32")]
pub mod weblock;
