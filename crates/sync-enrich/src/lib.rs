//! Durable podcast mutations shared by native and browser sync.
mod operation;
pub use operation::OutboxOp;

mod changes;
mod journal;
mod query;
pub use changes::StoreChanges;
pub use journal::{JournalEntry, StoredEntry};
pub use query::{
    EpisodeOrder, EpisodeQuery, EpisodeQueryFilter, filter_count, filter_sort_paginate,
};

mod legacy;
