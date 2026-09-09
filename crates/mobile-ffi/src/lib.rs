//! UniFFI boundary for native local profiles, shared sync, and device logging.

use std::sync::Once;

use halogen_logging_client as logging;

uniffi::setup_scaffolding!();

/// Errors crossing the FFI boundary. One flat variant for now — Swift shows
/// the message; structured variants can be added as the surface grows.
#[derive(Debug, uniffi::Error)]
pub enum CoreError {
    Server { msg: String },
}

impl std::fmt::Display for CoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CoreError::Server { msg } => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for CoreError {}

/// Install the app tracing subscriber once; native shells drain its device-log ring.
#[uniffi::export]
pub fn init_core() {
    static INIT: Once = Once::new();
    INIT.call_once(logging::init);
}

/// One Rust-side captured log line — `logging-client`'s `LogLine` flattened for
/// the FFI. `target` is the emitting module (`halogen_server`, `halogen_rss`,
/// …), the viewer's app-vs-server source tag (web: the target column on
/// `/logs/device`).
#[derive(uniffi::Record)]
pub struct DeviceLogLine {
    /// Unix epoch milliseconds (UTC).
    pub ts_ms: i64,
    /// "Error" | "Warn" | "Info" | "Debug" | "Trace".
    pub level: String,
    pub target: String,
    pub msg: String,
}

/// Push the app's Device Logs capture settings down into the Rust ring so
/// disabled/filtered lines are never buffered. `level` is a `Level` name
/// ("Error" | "Warn" | "Info" | …); unknown values fall back to Info.
#[uniffi::export]
pub fn set_device_log_capture(enabled: bool, level: String) {
    logging::set_enabled(enabled);
    logging::set_level(logging::Level::from_str_or_default(&level));
}

/// Take the Rust-side lines captured since the last call (oldest first). The
/// Swift DeviceLog polls this and folds the lines into its own persisted ring
/// — the single log surface, like the web's unified `/logs/device` ring.
#[uniffi::export]
pub fn drain_device_log() -> Vec<DeviceLogLine> {
    logging::drain_pending()
        .into_iter()
        .map(|l| DeviceLogLine {
            ts_ms: l.ts_ms,
            level: l.level.as_str().to_string(),
            target: l.target,
            msg: l.msg,
        })
        .collect()
}

mod local;
pub use local::*;

mod sync;
pub use sync::*;
