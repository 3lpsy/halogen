//! In-process host for the full Halogen server — the native app's
//! **Embedded Server** mode (see `docs/internal/EMBEDDED_SERVER_FEATURE.md`).
//!
//! Composition mirrors `halogen-server`'s `main` step for step (config →
//! migrate → seed → reclaim → polling → `build_router` → serve), with the
//! differences an in-process host requires:
//!
//! - **Config is a struct literal** (no clap/env/TOML — those belong to a real
//!   server install), with the runtime overrides file still applied via
//!   [`halogen_config::Config::load_and_apply_overrides`] so the admin
//!   config-overrides UI works.
//! - **Loopback only**: binds `127.0.0.1` on an OS-assigned port (kept across
//!   in-process restarts via `SO_REUSEADDR`). Auth — not the transport — is
//!   the security boundary; every route but `/healthz` and login requires a
//!   JWT, and the admin password is generated (see [`secrets`]).
//! - **Restart is in-process**: `POST /api/v1/admin/server/restart` trips the
//!   same `RestartHandle` the binary uses, but here the supervisor drains,
//!   tears down (polling [`shutdown`](halogen_polling::PollingHandle::shutdown),
//!   pool close), re-reads the overrides file, and serves again — never
//!   `execv`.
//! - **No logging init**: the UI owns the global `tracing` subscriber; server
//!   events flow into it (and the device log) automatically.
//! - **WAL profile** DB opens ([`halogen_migrate::connect_and_migrate_wal`]):
//!   a restart briefly overlaps the draining pool with the next one on the
//!   same file.
//!
//! Everything is process-global and `Send`. The supervisor runs on its OWN
//! small tokio runtime (dedicated threads): boot must not depend on the
//! host's ambient async context, and server load stays off the UI scheduler.

mod config;
mod dirs;
mod secrets;
mod supervisor;

pub use dirs::EmbeddedDirs;
pub use secrets::Credentials;
pub use supervisor::{
    EmbeddedStatus, base_url, credentials, credentials_for, destroy, ensure_started,
    generate_credentials, has_credentials, recover_admin, recover_user, remember_user, status,
    stop, subscribe_status, wait_ready,
};

#[cfg(test)]
mod tests;
