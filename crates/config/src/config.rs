//! Resolve defaults < TOML < HALOGEN_* env < CLI < runtime overrides. Only allowlisted overrides are writable at
//! runtime; secrets, binding/identity, and override-file settings stay protected. Config endpoints replace the override
//! set. Save load/rejection diagnostics on Config for logging after startup.

use std::env;
use std::fmt;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

use chrono::NaiveDate;
use clap::Parser;
use serde::Deserialize;

/// The built-in `subscription_no_sync_before` default: the sync service ignores
/// episodes published before this day unless overridden.
fn default_no_sync_before() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 1, 1).expect("valid default no_sync_before date")
}

/// Parse a `YYYY-MM-DD` config value into a date, warning (and keeping the
/// previous value) on a malformed string rather than aborting startup.
pub(crate) fn parse_no_sync_before(s: &str) -> Option<NaiveDate> {
    match NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d") {
        Ok(d) => Some(d),
        Err(_) => {
            eprintln!("Ignoring invalid subscription.no_sync_before {s:?} (want YYYY-MM-DD)");
            None
        }
    }
}

use crate::overrides::resolve_overrides_path;
use halogen_utils::constants::{
    DEFAULT_AUTO_DOWNLOAD_EPISODES_ENABLED, DEFAULT_AUTO_PLAYLIST_ADD_TO_START, DEFAULT_DB_PATH,
    DEFAULT_DISCOVER_GPODDER_SEARCH_URL, DEFAULT_DISCOVER_ITUNES_SEARCH_URL,
    DEFAULT_EPISODE_PLAYBACK_COMPLETE_PERCENTAGE, DEFAULT_LISTEN_ADDRESS, DEFAULT_LISTEN_PORT,
    DEFAULT_MAX_CONCURRENT_DOWNLOADS, DEFAULT_MAX_EPISODES, DEFAULT_MAX_POLL_CONCURRENT,
    DEFAULT_MEDIA_ROOT, DEFAULT_POLL_INTERVAL_SECONDS, DEFAULT_POLL_WAKE_INTERVAL_SECONDS,
    DEFAULT_TOKEN_EXPIRY_MINUTES,
};

#[derive(Debug, Clone, Parser)]
#[command(name = "halogen-server")]
#[command(about = "Halogen Podcast Server")]
#[command(version)]
pub struct Cli {
    #[arg(short, long)]
    pub config: Option<PathBuf>,
    // server
    #[arg(long)]
    pub listen_address: Option<String>,
    #[arg(long)]
    pub listen_port: Option<u16>,
    #[arg(long)]
    pub server_disable_polling_service: bool,
    /// User-Agent for every outbound server fetch (feeds, media, art, chapters,
    /// search). Unset = each client's own `Halogen/<ver> (+purpose)` default.
    #[arg(long)]
    pub server_fetch_user_agent: Option<String>,
    // db
    #[arg(long)]
    pub db_path: Option<PathBuf>,
    #[arg(long)]
    pub db_no_migrate: bool,
    /// Disable WAL journaling (pin the rollback journal instead). For
    /// network-filesystem storage, where WAL's shared-memory file is unsafe.
    /// Actively converts an existing WAL DB back on the next boot.
    #[arg(long)]
    pub db_no_wal: bool,
    /// Don't create the default "Queue" playlist on startup.
    #[arg(long)]
    pub db_skip_default_playlist: bool,
    // media
    #[arg(long)]
    pub media_root: Option<PathBuf>,
    // cors: repeatable `--cors-allowed-origin https://app.example.com`. Empty =
    // permissive (dev). Only matters when the frontend is a different origin.
    #[arg(long = "cors-allowed-origin")]
    pub cors_allowed_origins: Vec<String>,
    // public directory server (serves a directory, default at "/", e.g. the frontend)
    #[arg(long)]
    pub enable_public_server: bool,
    #[arg(long)]
    pub public_root: Option<PathBuf>,
    #[arg(long)]
    pub public_url_path: Option<String>,
    // subscription
    #[arg(long)]
    pub subscription_fallback_poll_interval: Option<u64>,
    #[arg(long)]
    pub subscription_poll_wake_interval: Option<u64>,
    #[arg(long)]
    pub subscription_fallback_max_episodes: Option<usize>,
    #[arg(long)]
    pub subscription_max_concurrent_downloads: Option<usize>,
    #[arg(long)]
    pub subscription_max_poll_concurrent: Option<usize>,
    /// Seconds a download may stay `Downloading` before the watchdog treats it as
    /// orphaned and resets it to `DownloadError`.
    #[arg(long)]
    pub subscription_download_stuck_after: Option<u64>,
    /// Whole-call download attempts before an episode is retired as `DownloadBroken`.
    #[arg(long)]
    pub subscription_download_max_attempts: Option<usize>,
    /// Auto-download new episodes server-side during polling (global default;
    /// per-podcast config can override).
    #[arg(long, default_value = "false")]
    pub subscription_poll_auto_download_enabled: bool,
    /// Insert auto-added episodes at the START of their target playlists instead
    /// of appending at the end (global default; per-podcast auto-playlist config
    /// can override).
    #[arg(long, default_value = "false")]
    pub subscription_auto_playlist_add_to_start: bool,
    /// `YYYY-MM-DD`: the sync service ignores episodes published before this day.
    #[arg(long)]
    pub subscription_no_sync_before: Option<String>,
    #[arg(long, default_value = "false")]
    pub subscription_sync_on_start: bool,
    // auth
    #[arg(long)]
    pub auth_token_expiry_minutes: Option<u64>,
    #[arg(long)]
    pub auth_token_secret: Option<String>,
    // episode
    #[arg(long)]
    pub episode_playback_complete_percentage: Option<u16>,
    // log
    #[arg(long)]
    pub log_file: Option<PathBuf>,
    #[arg(long)]
    pub log_level: Option<String>,
    #[arg(long, default_value = "false")]
    pub log_target: bool,
    #[arg(long, default_value = "false")]
    pub log_file_name: bool,
    #[arg(long, default_value = "true")]
    pub log_line_number: bool,
    // admin
    #[arg(long)]
    pub admin_username: Option<String>,
    #[arg(long)]
    pub admin_password: Option<String>,
    #[arg(long)]
    pub admin_disable_seed: bool,
    // opml
    #[arg(long)]
    pub opml_file: Option<PathBuf>,
    // config overrides
    /// Disable loading the runtime config-overrides file (enabled by default).
    #[arg(long)]
    pub config_overrides_disable: bool,
    /// Path to the runtime config-overrides file. Defaults to
    /// `config.overrides.toml` beside `--config` (else in the config dir).
    #[arg(long)]
    pub config_overrides_path: Option<PathBuf>,
    #[arg(long)]
    pub dev_use_mock_download: bool,
    #[arg(long)]
    pub dev_seed_data: bool,
    /// Allow outbound fetches (feeds, art, episode media) to private/loopback hosts.
    /// Off by default (SSRF guard); enable for dev or a feed host on your LAN.
    #[arg(long)]
    pub allow_private_network: bool,
    /// Override directory endpoints, including local fixture servers for tests.
    #[arg(long)]
    pub discover_itunes_base_url: Option<String>,
    #[arg(long)]
    pub discover_gpodder_base_url: Option<String>,
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct ConfigFile {
    #[serde(default)]
    pub server: ServerConfigFile,
    #[serde(default)]
    pub db: DbConfigFile,
    #[serde(default)]
    pub media: MediaConfigFile,
    #[serde(default, rename = "public")]
    pub public_files: PublicConfigFile,
    #[serde(default)]
    pub subscription: SubscriptionConfigFile,
    #[serde(default)]
    pub auth: AuthConfigFile,
    #[serde(default)]
    pub episode: EpisodeConfigFile,
    #[serde(default)]
    pub log: LogConfigFile,
    #[serde(default)]
    pub admin: AdminConfigFile,
    #[serde(default)]
    pub opml: OpmlConfigFile,
    #[serde(default)]
    pub config_overrides: ConfigOverridesConfigFile,
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct ServerConfigFile {
    pub listen_address: Option<String>,
    pub listen_port: Option<u16>,
    pub disable_polling_service: Option<bool>,
    /// User-Agent for outbound server fetches. Absent = per-client defaults.
    pub fetch_user_agent: Option<String>,
    /// Allow outbound fetches to private/loopback hosts (SSRF guard off). Default off.
    pub allow_private_network: Option<bool>,
    /// Allowed CORS origins. Empty/absent = permissive (dev).
    pub cors_allowed_origins: Option<Vec<String>>,
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct DbConfigFile {
    pub path: Option<String>,
    pub no_migrate: Option<bool>,
    /// Disable WAL journaling (see the `--db-no-wal` CLI doc).
    pub no_wal: Option<bool>,
    /// Skip creating the default "Queue" playlist on startup.
    pub skip_default_playlist: Option<bool>,
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct MediaConfigFile {
    pub root: Option<String>,
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct PublicConfigFile {
    pub enable: Option<bool>,
    pub root: Option<String>,
    pub url_path: Option<String>,
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct SubscriptionConfigFile {
    pub fallback_poll_interval: Option<u64>,
    pub poll_wake_interval: Option<u64>,
    pub fallback_max_episodes: Option<usize>,
    pub max_concurrent_downloads: Option<usize>,
    pub max_poll_concurrent: Option<usize>,
    /// Seconds a download may stay `Downloading` before the watchdog resets it.
    pub download_stuck_after: Option<u64>,
    /// Whole-call download attempts before an episode is retired as `DownloadBroken`.
    pub download_max_attempts: Option<usize>,
    pub poll_auto_download_enabled: Option<bool>,
    /// Insert auto-added episodes at the start of their target playlists.
    pub auto_playlist_add_to_start: Option<bool>,
    /// `YYYY-MM-DD`: skip episodes published before this day.
    pub no_sync_before: Option<String>,
    pub sync_on_start: Option<bool>,
    pub dev_use_mock_download: Option<bool>,
    pub dev_seed_data: Option<bool>,
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct AuthConfigFile {
    pub token_expiry_minutes: Option<u64>,
    pub token_secret: Option<String>,
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct EpisodeConfigFile {
    pub playback_complete_percentage: Option<u16>,
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct LogConfigFile {
    pub file: Option<String>,
    pub level: Option<String>,
    pub target: Option<bool>,
    pub file_name: Option<bool>,
    pub line_number: Option<bool>,
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct AdminConfigFile {
    pub username: Option<String>,
    pub password: Option<String>,
    pub disable_seed: Option<bool>,
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct OpmlConfigFile {
    pub file: Option<String>,
}

/// `[config_overrides]` — the knobs that govern the overrides mechanism itself.
/// These are intentionally NOT in the overrides allowlist (a file can't disable
/// or redirect its own loading).
#[derive(Deserialize, Debug, Clone, Default)]
pub struct ConfigOverridesConfigFile {
    pub disable: Option<bool>,
    pub path: Option<String>,
}

impl ConfigFile {
    pub fn from_path(path: &Path) -> Result<Self, String> {
        let bytes = fs::read(path).map_err(|e| format!("Could not read config file: {}", e))?;
        let s = String::from_utf8(bytes)
            .map_err(|e| format!("Config file is not valid UTF-8: {}", e))?;
        toml::from_str(&s).map_err(|e| format!("Failed to parse config file: {}", e))
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub listen_address: String,
    pub listen_port: u16,
    pub server_disable_polling_service: bool,
    /// User-Agent for EVERY outbound fetch (feed, download, chapters, art,
    /// discover). `None` = each client's own `Halogen/<ver> (+purpose)` default.
    /// Read once when each client is built, so it is startup-only — not a
    /// runtime override.
    pub server_fetch_user_agent: Option<String>,
    /// Allow outbound fetches (feeds, art, media) to private/loopback hosts. Off by
    /// default — the SSRF guard blocks them. Enable for dev / a LAN feed host.
    pub allow_private_network: bool,
    pub db_path: PathBuf,
    pub media_root: PathBuf,
    /// CORS allowed origins. Empty = permissive (reflect any origin) for dev.
    pub cors_allowed_origins: Vec<String>,
    pub enable_public_server: bool,
    pub public_root: Option<PathBuf>,
    pub public_url_path: String,
    pub subscription_fallback_poll_interval: Duration,
    pub subscription_poll_wake_interval: Duration,
    pub subscription_fallback_max_episodes: usize,
    pub subscription_max_concurrent_downloads: usize,
    pub subscription_max_poll_concurrent: usize,
    /// A download left `Downloading` longer than this is treated as orphaned and
    /// reset by the stuck-download watchdog. Boot config (`--subscription-download-stuck-after`
    /// / `HALOGEN_SUBSCRIPTION_DOWNLOAD_STUCK_AFTER`, seconds / `[subscription] download_stuck_after`).
    pub subscription_download_stuck_after: Duration,
    /// Attempt budget before a repeatedly-failing download is marked `DownloadBroken`.
    /// Boot config (`--subscription-download-max-attempts` /
    /// `HALOGEN_SUBSCRIPTION_DOWNLOAD_MAX_ATTEMPTS` / `[subscription] download_max_attempts`).
    pub subscription_download_max_attempts: usize,
    /// Global default for auto-downloading new episodes server-side during polling.
    /// Per-podcast `podcast_config.auto_download_enabled` overrides this.
    pub subscription_poll_auto_download_enabled: bool,
    /// Global default for whether the poller inserts auto-added episodes at the
    /// START of their target playlists (instead of appending at the end).
    /// Per-podcast `podcast_auto_playlist.add_to_start` overrides this.
    pub subscription_auto_playlist_add_to_start: bool,
    /// The sync service ignores episodes published before this day. Default
    /// 2026-01-01 so a fresh large OPML import doesn't pull years of back catalog.
    pub subscription_no_sync_before: NaiveDate,
    pub auth_token_expiry_minutes: u64,
    pub auth_token_secret: String,
    /// Episode is `Finished` once playback reaches the last N% of its duration.
    pub episode_playback_complete_percentage: u16,
    pub log_file: Option<PathBuf>,
    pub log_level: String,
    pub log_target: bool,
    pub log_file_name: bool,
    pub log_line_number: bool,
    pub db_no_migrate: bool,
    /// Disable WAL journaling for the SQLite DB (the server then pins the rollback journal, converting an
    /// existing WAL file back). Escape hatch for network-filesystem storage where WAL's shared-memory file is
    /// unsafe; WAL is the default — readers don't block the poll tick's writes. Boot-only (not
    /// runtime-overridable).
    pub db_no_wal: bool,
    /// Skip creating the default "Queue" playlist on startup.
    pub db_skip_default_playlist: bool,
    pub admin_username: Option<String>,
    pub admin_password: Option<String>,
    pub admin_disable_seed: bool,
    pub opml_file: Option<PathBuf>,
    pub subscription_sync_on_start: bool,
    pub dev_use_mock_download: bool,
    pub dev_seed_data: bool,
    // ── Discover providers ────────────────────────────────────────────────
    /// Upstream search endpoint for the iTunes discover provider. Defaults to the
    /// real Apple endpoint; integration tests point it at a wiremock server.
    pub discover_itunes_base_url: String,
    /// Upstream search endpoint for the gpodder.net discover provider. Defaults to
    /// the real endpoint; integration tests point it at a wiremock server.
    pub discover_gpodder_base_url: String,
    // ── Config overrides ──────────────────────────────────────────────────
    /// Whether loading the overrides file is disabled (default false = enabled).
    pub config_overrides_disable: bool,
    /// Resolved path the overrides file is read from / written to. Set during
    /// `resolve()`: explicit `config_overrides_path` > sibling of `--config` >
    /// config dir. `None` only when no config dir could be determined.
    pub config_overrides_path: Option<PathBuf>,
    /// Allowlisted field names the overrides file actually changed. Recorded
    /// here (logging isn't up yet during `resolve`) and logged from `main`.
    pub overridden_fields: Vec<String>,
    /// Known but non-overridable keys present in the overrides file — warned
    /// about after logging init, never fatal.
    pub rejected_override_keys: Vec<String>,
    /// The overrides file actually loaded, if any (vs. just the resolved path).
    pub config_overrides_loaded_from: Option<PathBuf>,
    /// A read/parse error from the overrides file, surfaced as a post-init warn.
    pub config_overrides_load_error: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen_address: DEFAULT_LISTEN_ADDRESS.to_string(),
            listen_port: DEFAULT_LISTEN_PORT,
            server_disable_polling_service: false,
            server_fetch_user_agent: None,
            allow_private_network: false,
            db_path: PathBuf::from(DEFAULT_DB_PATH),
            media_root: PathBuf::from(DEFAULT_MEDIA_ROOT),
            cors_allowed_origins: Vec::new(),
            enable_public_server: false,
            public_root: None,
            public_url_path: "/".to_string(),
            subscription_fallback_poll_interval: Duration::from_secs(
                DEFAULT_POLL_INTERVAL_SECONDS as u64,
            ),
            subscription_poll_wake_interval: Duration::from_secs(
                DEFAULT_POLL_WAKE_INTERVAL_SECONDS,
            ),
            subscription_fallback_max_episodes: DEFAULT_MAX_EPISODES as usize,
            subscription_max_concurrent_downloads: DEFAULT_MAX_CONCURRENT_DOWNLOADS as usize,
            subscription_max_poll_concurrent: DEFAULT_MAX_POLL_CONCURRENT as usize,
            subscription_download_stuck_after:
                halogen_utils::constants::DEFAULT_DOWNLOAD_STUCK_AFTER,
            subscription_download_max_attempts:
                halogen_utils::constants::DEFAULT_DOWNLOAD_MAX_ATTEMPTS,
            subscription_poll_auto_download_enabled: DEFAULT_AUTO_DOWNLOAD_EPISODES_ENABLED,
            subscription_auto_playlist_add_to_start: DEFAULT_AUTO_PLAYLIST_ADD_TO_START,
            subscription_no_sync_before: default_no_sync_before(),
            auth_token_expiry_minutes: DEFAULT_TOKEN_EXPIRY_MINUTES,
            episode_playback_complete_percentage: DEFAULT_EPISODE_PLAYBACK_COMPLETE_PERCENTAGE,
            // No default: required, validated in `resolve()`. Empty here so a
            // missing secret is caught rather than silently randomized per boot
            // (which invalidated every session on restart).
            auth_token_secret: String::new(),
            log_file: None,
            log_level: "info".to_string(),
            log_target: false,
            log_line_number: true,
            log_file_name: true,
            db_no_migrate: false,
            db_no_wal: false,
            db_skip_default_playlist: false,
            admin_username: None,
            admin_password: None,
            admin_disable_seed: false,
            opml_file: None,
            subscription_sync_on_start: false,
            dev_use_mock_download: false,
            dev_seed_data: false,
            discover_itunes_base_url: DEFAULT_DISCOVER_ITUNES_SEARCH_URL.to_string(),
            discover_gpodder_base_url: DEFAULT_DISCOVER_GPODDER_SEARCH_URL.to_string(),
            config_overrides_disable: false,
            config_overrides_path: None,
            overridden_fields: Vec::new(),
            rejected_override_keys: Vec::new(),
            config_overrides_loaded_from: None,
            config_overrides_load_error: None,
        }
    }
}

pub(crate) fn get_xdg_config_path() -> Option<PathBuf> {
    let base = directories::ProjectDirs::from("org", "fgsec", "halogen-server")?;
    Some(base.config_dir().join("halogen.toml"))
}

impl Config {
    /// Layer the config sources (defaults < TOML < env < CLI < overrides file) into a final [`Config`].
    /// Returns `Err` if a referenced config file is missing/invalid, or if `auth_token_secret` is empty after
    /// layering — it has no default and is required, since it signs JWTs and must stay stable across restarts
    /// to keep sessions valid.
    pub fn resolve(cli: &Cli) -> Result<Self, String> {
        let mut cfg = Self::default();

        if let Some(ref cli_config) = cli.config {
            if cli_config.exists() {
                let file = ConfigFile::from_path(cli_config)?;
                cfg.apply_config_file(&file);
            } else {
                return Err(format!("Config file not found: {}", cli_config.display()));
            }
        } else if let Some(xdg_config) = get_xdg_config_path()
            && xdg_config.exists()
        {
            let file = ConfigFile::from_path(&xdg_config)?;
            cfg.apply_config_file(&file);
        }

        cfg.apply_env();
        cfg.apply_cli(cli);

        // Config overrides are layered LAST — on top of CLI — so a runtime override file wins over boot config.
        // The `config_overrides_*` knobs are resolved through the normal chain above (they are not themselves
        // overridable), so they're known by now. Logging isn't initialized yet, so anything noteworthy is
        // recorded on `cfg` and logged from `main`.
        let configured = cfg.config_overrides_path.take();
        cfg.config_overrides_path = resolve_overrides_path(cli, configured.as_deref());
        if !cfg.config_overrides_disable
            && let Some(path) = cfg.config_overrides_path.clone()
            && path.exists()
        {
            match ConfigFile::from_path(&path) {
                Ok(file) => {
                    cfg.apply_overrides(&file);
                    cfg.config_overrides_loaded_from = Some(path);
                }
                Err(e) => cfg.config_overrides_load_error = Some(e),
            }
        }

        if cfg.auth_token_secret.is_empty() {
            return Err("auth token secret is required: set --auth-token-secret, \
                HALOGEN_AUTH_TOKEN_SECRET, or auth.token_secret in the config file"
                .to_string());
        }

        // Clamp the token expiry to a sane range regardless of source (TOML / env / CLI / overrides file — the
        // runtime `POST /config-overrides` path is validated by the DTO). `0` mints already-expired tokens
        // (total lockout), and a value near `u64::MAX` overflows `minutes * 60` / wraps the `i64` `exp` at mint
        // time. Floor 1 minute, ceiling ~100 years.
        const MAX_TOKEN_EXPIRY_MINUTES: u64 = 52_560_000; // ~100 years
        let clamped = cfg
            .auth_token_expiry_minutes
            .clamp(1, MAX_TOKEN_EXPIRY_MINUTES);
        if clamped != cfg.auth_token_expiry_minutes {
            eprintln!(
                "warning: auth_token_expiry_minutes {} is out of range; clamped to {}",
                cfg.auth_token_expiry_minutes, clamped
            );
            cfg.auth_token_expiry_minutes = clamped;
        }

        Ok(cfg)
    }

    fn apply_config_file(&mut self, file: &ConfigFile) {
        // `file.<section>.<field>` (Option) → `self.<field>`, by shape. `copy` is for
        // `Copy` fields (numbers/bools); `clone`/`path`/`opt_*` borrow. The
        // `no_sync_before` custom parser stays inline.
        macro_rules! copy {
            ($src:expr => $dst:ident) => {
                if let Some(v) = $src {
                    self.$dst = v;
                }
            };
        }
        macro_rules! clone {
            ($src:expr => $dst:ident) => {
                if let Some(v) = &$src {
                    self.$dst = v.clone();
                }
            };
        }
        macro_rules! path {
            ($src:expr => $dst:ident) => {
                if let Some(v) = &$src {
                    self.$dst = PathBuf::from(v);
                }
            };
        }
        macro_rules! opt_path {
            ($src:expr => $dst:ident) => {
                if let Some(v) = &$src {
                    self.$dst = Some(PathBuf::from(v));
                }
            };
        }
        macro_rules! opt_clone {
            ($src:expr => $dst:ident) => {
                if let Some(v) = &$src {
                    self.$dst = Some(v.clone());
                }
            };
        }
        macro_rules! secs {
            ($src:expr => $dst:ident) => {
                if let Some(v) = $src {
                    self.$dst = Duration::from_secs(v);
                }
            };
        }

        clone!(file.server.listen_address => listen_address);
        copy!(file.server.listen_port => listen_port);
        copy!(file.server.disable_polling_service => server_disable_polling_service);
        opt_clone!(file.server.fetch_user_agent => server_fetch_user_agent);
        path!(file.db.path => db_path);
        path!(file.media.root => media_root);
        clone!(file.server.cors_allowed_origins => cors_allowed_origins);
        copy!(file.public_files.enable => enable_public_server);
        opt_path!(file.public_files.root => public_root);
        clone!(file.public_files.url_path => public_url_path);
        secs!(file.subscription.fallback_poll_interval => subscription_fallback_poll_interval);
        secs!(file.subscription.poll_wake_interval => subscription_poll_wake_interval);
        copy!(file.subscription.fallback_max_episodes => subscription_fallback_max_episodes);
        copy!(file.subscription.max_concurrent_downloads => subscription_max_concurrent_downloads);
        copy!(file.subscription.max_poll_concurrent => subscription_max_poll_concurrent);
        secs!(file.subscription.download_stuck_after => subscription_download_stuck_after);
        copy!(file.subscription.download_max_attempts => subscription_download_max_attempts);
        copy!(file.subscription.poll_auto_download_enabled => subscription_poll_auto_download_enabled);
        copy!(file.subscription.auto_playlist_add_to_start => subscription_auto_playlist_add_to_start);
        if let Some(v) = &file.subscription.no_sync_before
            && let Some(d) = parse_no_sync_before(v)
        {
            self.subscription_no_sync_before = d;
        }
        copy!(file.auth.token_expiry_minutes => auth_token_expiry_minutes);
        copy!(file.episode.playback_complete_percentage => episode_playback_complete_percentage);
        clone!(file.auth.token_secret => auth_token_secret);
        opt_path!(file.log.file => log_file);
        clone!(file.log.level => log_level);
        copy!(file.log.target => log_target);
        copy!(file.log.file_name => log_file_name);
        copy!(file.log.line_number => log_line_number);
        copy!(file.db.no_migrate => db_no_migrate);
        copy!(file.db.no_wal => db_no_wal);
        copy!(file.db.skip_default_playlist => db_skip_default_playlist);
        opt_clone!(file.admin.username => admin_username);
        opt_clone!(file.admin.password => admin_password);
        copy!(file.admin.disable_seed => admin_disable_seed);
        opt_path!(file.opml.file => opml_file);
        copy!(file.config_overrides.disable => config_overrides_disable);
        opt_path!(file.config_overrides.path => config_overrides_path);
        copy!(file.subscription.sync_on_start => subscription_sync_on_start);
        copy!(file.subscription.dev_use_mock_download => dev_use_mock_download);
        copy!(file.subscription.dev_seed_data => dev_seed_data);
        copy!(file.server.allow_private_network => allow_private_network);
    }

    fn apply_env(&mut self) {
        // Each `HALOGEN_*` var maps to one field by shape. The macros collapse the four mechanical patterns
        // (set string / set Some-string / set PathBuf / `parse()`); the two genuinely special cases (CORS
        // list-split, the `no_sync_before` custom parser) stay inline below. Adding a field is one line, not a
        // six-line `if let` block.
        macro_rules! env_str {
            ($var:literal, $field:ident) => {
                if let Some(v) = config_env_var($var)
                    && !v.is_empty()
                {
                    self.$field = v;
                }
            };
        }
        macro_rules! env_opt_str {
            ($var:literal, $field:ident) => {
                if let Some(v) = config_env_var($var)
                    && !v.is_empty()
                {
                    self.$field = Some(v);
                }
            };
        }
        macro_rules! env_path {
            ($var:literal, $field:ident) => {
                if let Some(v) = config_env_var($var)
                    && !v.is_empty()
                {
                    self.$field = PathBuf::from(v);
                }
            };
        }
        macro_rules! env_opt_path {
            ($var:literal, $field:ident) => {
                if let Some(v) = config_env_var($var)
                    && !v.is_empty()
                {
                    self.$field = Some(PathBuf::from(v));
                }
            };
        }
        macro_rules! env_parse {
            ($var:literal, $field:ident) => {
                if let Some(v) = config_env_var($var)
                    && let Ok(v) = v.parse()
                {
                    self.$field = v;
                }
            };
        }
        macro_rules! env_secs {
            ($var:literal, $field:ident) => {
                if let Some(v) = config_env_var($var)
                    && let Ok(v) = v.parse()
                {
                    self.$field = Duration::from_secs(v);
                }
            };
        }

        env_str!("HALOGEN_LISTEN_ADDRESS", listen_address);
        env_parse!("HALOGEN_LISTEN_PORT", listen_port);
        env_parse!(
            "HALOGEN_SERVER_DISABLE_POLLING_SERVICE",
            server_disable_polling_service
        );
        env_opt_str!("HALOGEN_SERVER_FETCH_USER_AGENT", server_fetch_user_agent);
        env_path!("HALOGEN_DB_PATH", db_path);
        env_path!("HALOGEN_MEDIA_ROOT", media_root);
        if let Some(v) = config_env_var("HALOGEN_CORS_ALLOWED_ORIGINS")
            && !v.is_empty()
        {
            // Comma-separated list of origins.
            self.cors_allowed_origins = v
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
        }
        env_parse!("HALOGEN_ENABLE_PUBLIC_SERVER", enable_public_server);
        env_opt_path!("HALOGEN_PUBLIC_ROOT", public_root);
        env_str!("HALOGEN_PUBLIC_URL_PATH", public_url_path);
        env_secs!(
            "HALOGEN_SUBSCRIPTION_FALLBACK_POLL_INTERVAL",
            subscription_fallback_poll_interval
        );
        env_secs!(
            "HALOGEN_SUBSCRIPTION_POLL_WAKE_INTERVAL",
            subscription_poll_wake_interval
        );
        env_parse!(
            "HALOGEN_SUBSCRIPTION_FALLBACK_MAX_EPISODES",
            subscription_fallback_max_episodes
        );
        env_parse!(
            "HALOGEN_SUBSCRIPTION_MAX_CONCURRENT_DOWNLOADS",
            subscription_max_concurrent_downloads
        );
        env_parse!(
            "HALOGEN_SUBSCRIPTION_MAX_POLL_CONCURRENT",
            subscription_max_poll_concurrent
        );
        env_secs!(
            "HALOGEN_SUBSCRIPTION_DOWNLOAD_STUCK_AFTER",
            subscription_download_stuck_after
        );
        env_parse!(
            "HALOGEN_SUBSCRIPTION_DOWNLOAD_MAX_ATTEMPTS",
            subscription_download_max_attempts
        );
        env_parse!(
            "HALOGEN_SUBSCRIPTION_POLL_AUTO_DOWNLOAD_ENABLED",
            subscription_poll_auto_download_enabled
        );
        env_parse!(
            "HALOGEN_SUBSCRIPTION_AUTO_PLAYLIST_ADD_TO_START",
            subscription_auto_playlist_add_to_start
        );
        if let Some(v) = config_env_var("HALOGEN_SUBSCRIPTION_NO_SYNC_BEFORE")
            && let Some(d) = parse_no_sync_before(&v)
        {
            self.subscription_no_sync_before = d;
        }
        env_parse!(
            "HALOGEN_AUTH_TOKEN_EXPIRY_MINUTES",
            auth_token_expiry_minutes
        );
        env_parse!(
            "HALOGEN_EPISODE_PLAYBACK_COMPLETE_PERCENTAGE",
            episode_playback_complete_percentage
        );
        env_opt_path!("HALOGEN_LOG_FILE", log_file);
        env_str!("HALOGEN_LOG_LEVEL", log_level);
        env_parse!("HALOGEN_LOG_TARGET", log_target);
        env_parse!("HALOGEN_LOG_FILE_NAME", log_file_name);
        env_parse!("HALOGEN_LOG_LINE_NUMBER", log_line_number);
        env_parse!("HALOGEN_DB_NO_MIGRATE", db_no_migrate);
        env_parse!("HALOGEN_DB_NO_WAL", db_no_wal);
        env_parse!("HALOGEN_DB_SKIP_DEFAULT_PLAYLIST", db_skip_default_playlist);
        env_opt_str!("HALOGEN_ADMIN_USERNAME", admin_username);
        env_opt_str!("HALOGEN_ADMIN_PASSWORD", admin_password);
        env_parse!("HALOGEN_ADMIN_DISABLE_SEED", admin_disable_seed);
        env_str!("HALOGEN_AUTH_TOKEN_SECRET", auth_token_secret);
        env_opt_path!("HALOGEN_OPML_FILE", opml_file);
        env_parse!("HALOGEN_CONFIG_OVERRIDES_DISABLE", config_overrides_disable);
        env_opt_path!("HALOGEN_CONFIG_OVERRIDES_PATH", config_overrides_path);
        env_parse!(
            "HALOGEN_SUBSCRIPTION_SYNC_ON_START",
            subscription_sync_on_start
        );
        env_parse!("HALOGEN_DEV_USE_MOCK_DOWNLOAD", dev_use_mock_download);
        env_parse!("HALOGEN_DEV_SEED_DATA", dev_seed_data);
        env_parse!("HALOGEN_ALLOW_PRIVATE_NETWORK", allow_private_network);
        env_str!("HALOGEN_DISCOVER_ITUNES_BASE_URL", discover_itunes_base_url);
        env_str!(
            "HALOGEN_DISCOVER_GPODDER_BASE_URL",
            discover_gpodder_base_url
        );
    }

    fn apply_cli(&mut self, cli: &Cli) {
        // `flag` = clap `store_true` bool (only ever sets, never clears); `copy` =
        // `Copy` Option; `clone`/`opt_clone` borrow an Option. The CORS list and the
        // `no_sync_before` custom parser stay inline.
        macro_rules! flag {
            ($src:expr => $dst:ident) => {
                if $src {
                    self.$dst = true;
                }
            };
        }
        macro_rules! copy {
            ($src:expr => $dst:ident) => {
                if let Some(v) = $src {
                    self.$dst = v;
                }
            };
        }
        macro_rules! clone {
            ($src:expr => $dst:ident) => {
                if let Some(v) = &$src {
                    self.$dst = v.clone();
                }
            };
        }
        macro_rules! opt_clone {
            ($src:expr => $dst:ident) => {
                if let Some(v) = &$src {
                    self.$dst = Some(v.clone());
                }
            };
        }
        macro_rules! secs {
            ($src:expr => $dst:ident) => {
                if let Some(v) = $src {
                    self.$dst = Duration::from_secs(v);
                }
            };
        }

        clone!(cli.listen_address => listen_address);
        copy!(cli.listen_port => listen_port);
        flag!(cli.server_disable_polling_service => server_disable_polling_service);
        opt_clone!(cli.server_fetch_user_agent => server_fetch_user_agent);
        clone!(cli.db_path => db_path);
        clone!(cli.media_root => media_root);
        if !cli.cors_allowed_origins.is_empty() {
            self.cors_allowed_origins = cli.cors_allowed_origins.clone();
        }
        flag!(cli.enable_public_server => enable_public_server);
        opt_clone!(cli.public_root => public_root);
        clone!(cli.public_url_path => public_url_path);
        secs!(cli.subscription_fallback_poll_interval => subscription_fallback_poll_interval);
        secs!(cli.subscription_poll_wake_interval => subscription_poll_wake_interval);
        copy!(cli.subscription_fallback_max_episodes => subscription_fallback_max_episodes);
        copy!(cli.subscription_max_concurrent_downloads => subscription_max_concurrent_downloads);
        copy!(cli.subscription_max_poll_concurrent => subscription_max_poll_concurrent);
        secs!(cli.subscription_download_stuck_after => subscription_download_stuck_after);
        copy!(cli.subscription_download_max_attempts => subscription_download_max_attempts);
        flag!(cli.subscription_poll_auto_download_enabled => subscription_poll_auto_download_enabled);
        flag!(cli.subscription_auto_playlist_add_to_start => subscription_auto_playlist_add_to_start);
        if let Some(v) = &cli.subscription_no_sync_before
            && let Some(d) = parse_no_sync_before(v)
        {
            self.subscription_no_sync_before = d;
        }
        copy!(cli.auth_token_expiry_minutes => auth_token_expiry_minutes);
        copy!(cli.episode_playback_complete_percentage => episode_playback_complete_percentage);
        opt_clone!(cli.log_file => log_file);
        clone!(cli.log_level => log_level);
        flag!(cli.log_target => log_target);
        flag!(cli.log_file_name => log_file_name);
        flag!(cli.log_line_number => log_line_number);
        flag!(cli.db_no_migrate => db_no_migrate);
        flag!(cli.db_no_wal => db_no_wal);
        flag!(cli.db_skip_default_playlist => db_skip_default_playlist);
        opt_clone!(cli.admin_username => admin_username);
        opt_clone!(cli.admin_password => admin_password);
        flag!(cli.admin_disable_seed => admin_disable_seed);
        opt_clone!(cli.opml_file => opml_file);
        flag!(cli.config_overrides_disable => config_overrides_disable);
        opt_clone!(cli.config_overrides_path => config_overrides_path);
        flag!(cli.subscription_sync_on_start => subscription_sync_on_start);
        clone!(cli.auth_token_secret => auth_token_secret);
        flag!(cli.dev_use_mock_download => dev_use_mock_download);
        flag!(cli.dev_seed_data => dev_seed_data);
        flag!(cli.allow_private_network => allow_private_network);
        clone!(cli.discover_itunes_base_url => discover_itunes_base_url);
        clone!(cli.discover_gpodder_base_url => discover_gpodder_base_url);
    }
}

pub fn build() -> Result<Config, String> {
    let cli = Cli::parse();
    Config::resolve(&cli)
}

#[cfg(not(test))]
fn config_env_var(name: &str) -> Option<String> {
    env::var(name).ok()
}

#[cfg(test)]
fn config_env_var(name: &str) -> Option<String> {
    TEST_KEY.with(|key| {
        let k = key.borrow();
        let key_str = k.as_deref().unwrap_or("");
        if !key_str.is_empty()
            && let Ok(v) = env::var(format!("{name}_{key_str}"))
        {
            return Some(v);
        }
        if let Ok(v) = env::var(name) {
            return Some(v);
        }
        // Tests resolve config without supplying a secret; prod requires one.
        if name == "HALOGEN_AUTH_TOKEN_SECRET" {
            return Some("test-secret".to_string());
        }
        None
    })
}

#[cfg(test)]
use std::cell::RefCell;

#[cfg(test)]
thread_local! {
    static TEST_KEY: RefCell<Option<String>> = const { RefCell::new(None) };
}

#[cfg(test)]
pub fn set_test_env_key(key: &str) {
    TEST_KEY.with(|k| k.borrow_mut().replace(key.to_string()));
}

impl fmt::Display for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Halogen Server Config")?;
        writeln!(f, "  listen_address: {}", self.listen_address)?;
        writeln!(f, "  listen_port: {}", self.listen_port)?;
        writeln!(
            f,
            "  server_disable_polling_service: {}",
            self.server_disable_polling_service
        )?;
        writeln!(f, "  db_path: {}", self.db_path.display())?;
        writeln!(f, "  db_no_wal: {}", self.db_no_wal)?;
        writeln!(
            f,
            "  db_skip_default_playlist: {}",
            self.db_skip_default_playlist
        )?;
        writeln!(f, "  media_root: {}", self.media_root.display())?;
        writeln!(f, "  cors_allowed_origins: {:?}", self.cors_allowed_origins)?;
        writeln!(f, "  enable_public_server: {}", self.enable_public_server)?;
        writeln!(
            f,
            "  public_root: {:?}",
            self.public_root
                .as_ref()
                .map(|p| p.to_string_lossy().to_string())
        )?;
        writeln!(f, "  public_url_path: {}", self.public_url_path)?;
        writeln!(
            f,
            "  fallback_poll_interval: {:?}",
            self.subscription_fallback_poll_interval
        )?;
        writeln!(
            f,
            "  poll_wake_interval: {:?}",
            self.subscription_poll_wake_interval
        )?;
        writeln!(
            f,
            "  fallback_max_episodes: {}",
            self.subscription_fallback_max_episodes
        )?;
        writeln!(
            f,
            "  max_concurrent_downloads: {}",
            self.subscription_max_concurrent_downloads
        )?;
        writeln!(
            f,
            "  max_poll_concurrent: {}",
            self.subscription_max_poll_concurrent
        )?;
        writeln!(
            f,
            "  poll_auto_download_enabled: {}",
            self.subscription_poll_auto_download_enabled
        )?;
        writeln!(
            f,
            "  auto_playlist_add_to_start: {}",
            self.subscription_auto_playlist_add_to_start
        )?;
        writeln!(f, "  no_sync_before: {}", self.subscription_no_sync_before)?;
        writeln!(f, "  dev_use_mock_download: {}", self.dev_use_mock_download)?;
        writeln!(f, "  dev_seed_data: {}", self.dev_seed_data)?;
        writeln!(f, "  allow_private_network: {}", self.allow_private_network)?;
        writeln!(
            f,
            "  server_fetch_user_agent: {}",
            self.server_fetch_user_agent
                .as_deref()
                .unwrap_or("<default>")
        )?;
        writeln!(
            f,
            "  token_expiry: {} days",
            self.auth_token_expiry_minutes / 60 / 24
        )?;
        writeln!(
            f,
            "  log_file: {:?}",
            self.log_file
                .as_ref()
                .map(|p| p.to_string_lossy().to_string())
        )?;
        writeln!(f, "  log_level: {}", self.log_level)?;
        writeln!(f, "  log_target: {}", self.log_target)?;
        writeln!(f, "  log_line_number: {}", self.log_line_number)?;
        writeln!(
            f,
            "  config_overrides_disable: {}",
            self.config_overrides_disable
        )?;
        writeln!(
            f,
            "  config_overrides_path: {:?}",
            self.config_overrides_path
                .as_ref()
                .map(|p| p.to_string_lossy().to_string())
        )?;
        if !self.overridden_fields.is_empty() {
            writeln!(f, "  overridden_fields: {:?}", self.overridden_fields)?;
        }
        Ok(())
    }
}
