//! RSS feed sync: fetch podcast feeds, ingest new episodes, and — when enabled —
//! auto-download new episodes + enforce the per-podcast retention cap.
//!
//! [`RssManager`] is the orchestrator (owns the DB handle, HTTP client, and
//! resolved [`SyncContext`]); the re-exported free functions are the thin entry
//! points the poller and tests call. Feed parsing lives in [`feed`].

mod feed;
mod manager;
mod types;

pub use feed::parse_feed;
pub use manager::{RssManager, sync, sync_reported_with_context, sync_with_context};
pub use types::{RemoteEpisodeData, RemoteFeedData, SyncContext};

#[cfg(test)]
mod tests;
