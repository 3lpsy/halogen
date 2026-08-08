//! Shared, sanitised view of the server's reconciled runtime configuration.
//!
//! This is a plain data-transfer struct — **not** a database entity. It carries
//! the runtime config (after defaults < TOML < env < CLI < overrides file are
//! layered) from the server to API clients / the frontend for the admin "view
//! config" screen, including which fields the overrides file changed.
//!
//! It deliberately **omits secret fields** (the JWT signing secret / API token
//! and the admin password): they are never represented here, so they cannot be
//! serialised out. The server builds this from its internal `Config`; clients
//! deserialise it.

use serde::{Deserialize, Serialize};
use validator::Validate;

use super::ResponsableData;

/// Sanitised projection of the server's reconciled runtime config.
///
/// Every runtime setting is represented **except** the two secrets
/// (`auth_token_secret`, `admin_password`). Filesystem paths are strings and
/// `Duration`s are whole seconds for a stable JSON shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConfigData {
    pub listen_address: String,
    pub listen_port: u16,
    pub server_disable_polling_service: bool,
    pub db_path: String,
    pub db_no_migrate: bool,
    #[serde(default)]
    pub db_skip_default_playlist: bool,
    pub media_root: String,
    pub cors_allowed_origins: Vec<String>,
    pub enable_public_server: bool,
    pub public_root: Option<String>,
    pub public_url_path: String,
    pub subscription_fallback_poll_interval_secs: u64,
    pub subscription_poll_wake_interval_secs: u64,
    pub subscription_fallback_max_episodes: usize,
    pub subscription_max_concurrent_downloads: usize,
    pub subscription_max_poll_concurrent: usize,
    #[serde(default)]
    pub subscription_poll_auto_download_enabled: bool,
    /// Whether the poller inserts auto-added episodes at the START of their
    /// target playlists (instead of appending at the end).
    #[serde(default)]
    pub subscription_auto_playlist_add_to_start: bool,
    /// `YYYY-MM-DD`: the sync service ignores episodes published before this day.
    #[serde(default)]
    pub subscription_no_sync_before: String,
    pub subscription_sync_on_start: bool,
    pub auth_token_expiry_minutes: u64,
    pub episode_playback_complete_percentage: u16,
    pub log_file: Option<String>,
    pub log_level: String,
    pub log_target: bool,
    pub log_file_name: bool,
    pub log_line_number: bool,
    pub admin_username: Option<String>,
    pub admin_disable_seed: bool,
    pub opml_file: Option<String>,
    pub dev_use_mock_download: bool,
    pub dev_seed_data: bool,
    // ── Config overrides ──────────────────────────────────────────────────
    /// Names of the allowlisted fields the overrides file actually changed
    /// (after `defaults < TOML < env < CLI < overrides`).
    #[serde(default)]
    pub overridden_fields: Vec<String>,
    /// Whether the config-overrides mechanism is disabled for this process.
    #[serde(default)]
    pub config_overrides_disabled: bool,
    /// Resolved path the overrides file is read from / written to (where it
    /// *would* live even if no file exists yet).
    #[serde(default)]
    pub config_overrides_path: Option<String>,
    /// Whether an overrides file existed and was successfully loaded at boot.
    #[serde(default)]
    pub config_overrides_loaded: bool,
}

impl ResponsableData for ConfigData {}

/// The writable subset of runtime config (the allowlist) that may be supplied
/// via the overrides file / `POST /config-overrides`.
///
/// Every field is optional: a partial patch sets only the keys it carries, and
/// the persisted file likewise only lists overridden keys. Field names mirror
/// [`ConfigData`] (durations as whole seconds) for a stable JSON shape.
///
/// Deliberately **excludes** secrets (`auth_token_secret`, `admin_password`),
/// the `config_overrides_*` knobs themselves, and the run-once binding/identity
/// fields (listen address/port, db path, media root, migrations, admin seed).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Validate)]
pub struct ConfigOverridesData {
    pub subscription_fallback_poll_interval_secs: Option<u64>,
    pub subscription_poll_wake_interval_secs: Option<u64>,
    pub subscription_fallback_max_episodes: Option<usize>,
    pub subscription_max_concurrent_downloads: Option<usize>,
    pub subscription_max_poll_concurrent: Option<usize>,
    pub subscription_poll_auto_download_enabled: Option<bool>,
    /// Insert auto-added episodes at the start of their target playlists.
    pub subscription_auto_playlist_add_to_start: Option<bool>,
    /// `YYYY-MM-DD`: the sync service ignores episodes published before this day.
    pub subscription_no_sync_before: Option<String>,
    pub subscription_sync_on_start: Option<bool>,
    #[validate(range(
        min = 1,
        max = 52_560_000,
        message = "Token expiry must be between 1 minute and ~100 years"
    ))]
    pub auth_token_expiry_minutes: Option<u64>,
    #[validate(range(
        min = 0,
        max = 100,
        message = "Playback complete percentage must be between 0 and 100"
    ))]
    pub episode_playback_complete_percentage: Option<u16>,
    pub opml_file: Option<String>,
}

impl ResponsableData for ConfigOverridesData {}
