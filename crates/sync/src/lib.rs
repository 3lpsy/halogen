//! Shared durable push and incremental pull engines. Platform adapters own scheduling.
mod pull;
mod push;
pub use pull::{PullOutcome, cached_snapshot, pull_changes};
pub use push::{PushOutcome, push_entry};

#[cfg(test)]
mod tests;
