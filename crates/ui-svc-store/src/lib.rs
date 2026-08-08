//! Local metadata persistence. Re-export shell; the `LocalStore` trait +
//! `StoreHandle` live in [`store`], the query/filter model in `query`, the offline
//! op log in `outbox`, and the per-target backends in `native` (SQLite) / `web`
//! (localStorage).

pub mod outbox;
mod query;
mod store;

#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(target_arch = "wasm32")]
mod web;

#[cfg(not(target_arch = "wasm32"))]
pub use native::NativeLocalStore;
pub use outbox::OutboxOp;
pub use query::{EpisodeOrder, EpisodeQuery, EpisodeQueryFilter};
pub(crate) use query::{filter_count, filter_sort_paginate};
pub use store::{LocalStore, StoreHandle};
#[cfg(target_arch = "wasm32")]
pub use web::WebLocalStore;
