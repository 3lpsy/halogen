//! Browser persistence adapter for the shared sync-store contract.
pub mod outbox;
pub use halogen_sync_store::*;
#[cfg(target_arch = "wasm32")]
mod web;
#[cfg(target_arch = "wasm32")]
pub use web::WebLocalStore;
