//! The native-iOS FFI surface — what the SwiftUI app (`ios/`) calls.
//!
//! Scope today: embedded-server lifecycle (start / readiness / status), the
//! first proven Swift → Rust seam. The app boots the full in-process Axum
//! server through [`start_server`] and then talks to it over loopback HTTP
//! like any other client. Next slices (sync service, commands, state
//! snapshots) land here as the dioxus views are replaced.
//!
//! Everything exported here goes through UniFFI proc-macros; the Swift side
//! is generated (`just ios-core`), never hand-written.

use std::path::PathBuf;
use std::sync::Once;

use halogen_embedded_server as server;
use halogen_ui_logging as logging;

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

/// Snapshot of the embedded server's lifecycle for the Swift UI.
#[derive(uniffi::Record)]
pub struct ServerStatus {
    /// "stopped" | "starting" | "running" | "failed"
    pub state: String,
    pub port: Option<u16>,
    pub error: Option<String>,
}

/// Install the global `tracing` subscriber. Idempotent. The iOS app is the UI
/// host, so it owns the subscriber (same rule as the dioxus app): `ui-logging`'s
/// console layer (stderr, visible via `simctl launch --console`) plus its
/// device-log ring, which captures the embedded server's tracing — the app
/// drains it via [`drain_device_log`] into the native Device Logs screen.
#[uniffi::export]
pub fn init_core() {
    static INIT: Once = Once::new();
    INIT.call_once(logging::init);
}

/// One Rust-side captured log line — `ui-logging`'s `LogLine` flattened for
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

/// Boot (or reuse) the embedded server rooted at `data_root` — the app passes
/// a directory under its Application Support container. Returns the loopback
/// base URL. Synchronous by design: the supervisor runs on its own tokio
/// runtime, this only pokes it.
#[uniffi::export]
pub fn start_server(data_root: String) -> Result<String, CoreError> {
    server::ensure_started(server::EmbeddedDirs::new(PathBuf::from(data_root)))
        .map_err(|msg| CoreError::Server { msg })
}

/// Resolve when the server is serving; returns the bound port.
#[uniffi::export(async_runtime = "tokio")]
pub async fn wait_ready() -> Result<u16, CoreError> {
    server::wait_ready()
        .await
        .map_err(|msg| CoreError::Server { msg })
}

/// Loopback base URL once a start has been requested (`None` before).
#[uniffi::export]
pub fn base_url() -> Option<String> {
    server::base_url()
}

/// Silent-login credentials for the embedded library (the provisioned admin),
/// read from `secrets.json` under `data_root`. Errors until [`start_server`]
/// has provisioned the library. The Swift API client trades these for a JWT
/// at `/api/v1/auth/login` — auth, not the loopback transport, is the
/// security boundary.
#[derive(uniffi::Record)]
pub struct EmbeddedCredentials {
    pub username: String,
    pub password: String,
}

#[uniffi::export]
pub fn credentials(data_root: String) -> Result<EmbeddedCredentials, CoreError> {
    server::credentials(&server::EmbeddedDirs::new(PathBuf::from(data_root)))
        .map(|c| EmbeddedCredentials {
            username: c.username,
            password: c.password,
        })
        .map_err(|msg| CoreError::Server { msg })
}

/// Auth self-heal for a SPECIFIC embedded user: rotate their password
/// directly in the DB and in `secrets.json`, returning the fresh credentials.
/// Covers silent-login drift and re-keys users a DB import created with
/// random passwords (web: `embedded::recover_user` after an import). The row
/// must exist; safe against a running server (password-only rotation).
#[uniffi::export(async_runtime = "tokio")]
pub async fn recover_user(
    data_root: String,
    username: String,
) -> Result<EmbeddedCredentials, CoreError> {
    server::recover_user(
        &server::EmbeddedDirs::new(PathBuf::from(data_root)),
        &username,
    )
    .await
    .map(|c| EmbeddedCredentials {
        username: c.username,
        password: c.password,
    })
    .map_err(|msg| CoreError::Server { msg })
}

/// Auth self-heal for the embedded ADMIN: rotate the admin row's password in
/// the DB + `secrets.json`, ADOPTING the row's username when it was renamed
/// out from under the stored secrets (web: `embedded::recover_admin`). The
/// returned username is the row that actually exists — silent login must
/// target it.
#[uniffi::export(async_runtime = "tokio")]
pub async fn recover_admin(data_root: String) -> Result<EmbeddedCredentials, CoreError> {
    server::recover_admin(&server::EmbeddedDirs::new(PathBuf::from(data_root)))
        .await
        .map(|c| EmbeddedCredentials {
            username: c.username,
            password: c.password,
        })
        .map_err(|msg| CoreError::Server { msg })
}

/// Destroy the embedded library at `data_root`: stops the in-process server
/// (when it's serving this directory) and deletes the database, media, and
/// secrets. The Local Data purge screen's nuclear option.
#[uniffi::export(async_runtime = "tokio")]
pub async fn destroy_embedded(data_root: String) -> Result<(), CoreError> {
    server::destroy(&server::EmbeddedDirs::new(PathBuf::from(data_root)))
        .await
        .map_err(|msg| CoreError::Server { msg })
}

#[uniffi::export]
pub fn server_status() -> ServerStatus {
    match server::status() {
        server::EmbeddedStatus::Stopped => ServerStatus {
            state: "stopped".into(),
            port: None,
            error: None,
        },
        server::EmbeddedStatus::Starting { port } => ServerStatus {
            state: "starting".into(),
            port: Some(port),
            error: None,
        },
        server::EmbeddedStatus::Running { port } => ServerStatus {
            state: "running".into(),
            port: Some(port),
            error: None,
        },
        server::EmbeddedStatus::Failed { error } => ServerStatus {
            state: "failed".into(),
            port: None,
            error: Some(error),
        },
    }
}
