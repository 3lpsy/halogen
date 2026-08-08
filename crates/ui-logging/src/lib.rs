//! Logging: one `tracing` subscriber we own, on every target.
//!
//! Re-export shell. The device-log core (`Level`/`LogLine`, the in-memory ring,
//! the subscriber + `DeviceLogLayer` `init`, capture/snapshot/search/export) lives
//! in [`device_log`]; persistence (IndexedDB on web, a file on native) in [`store`].
//!
//! `logging::{info,warn,error,debug,trace}!` are thin re-exports of the `tracing`
//! macros (kept at the crate root so call sites are unchanged and structured
//! fields work). Capture happens in `DeviceLogLayer`, so these — and any direct
//! `tracing::*!`, including from dependencies — feed the device log uniformly.

mod device_log;
pub mod store;

pub(crate) use device_log::CAP;
pub use device_log::*;

#[allow(unused_imports)]
pub use tracing::{debug, error, info, trace, warn};
