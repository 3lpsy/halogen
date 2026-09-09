//! Device log capture shared by native shells and the web UI.
mod ring;
mod subscriber;
mod types;
pub use ring::{
    CAP, clear, drain_pending, enabled, export_text, ingest, ingest_persisted, level, search,
    set_enabled, set_level, snapshot,
};
pub use subscriber::{init, init_forwarding};
pub use tracing::{debug, error, info, trace, warn};
pub use types::{Level, LogLine};
#[cfg(test)]
mod tests;
