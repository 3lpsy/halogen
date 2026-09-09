//! Persist the device account registry in IndexedDB or native JSON and orchestrate account switching, sign-out, token
//! refresh, namespace changes, and local-data wipes.

pub mod account_actions;
pub mod accounts;
#[cfg(target_arch = "wasm32")]
mod web;
