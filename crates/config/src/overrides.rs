//! Runtime config-overrides file: the one writable-at-runtime layer.
//!
//! [`Config::apply_overrides`] layers an allowlisted overrides file on top of
//! the resolved config (rejecting secrets / identity / boot-only keys), and
//! [`read_overrides`] / [`write_overrides`] back the `/config-overrides` endpoints.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use halogen_utils::constants::DEFAULT_CONFIG_OVERRIDES_FILENAME;
use halogen_wire::ConfigOverridesData;

use crate::config::{Cli, Config, ConfigFile, get_xdg_config_path, parse_no_sync_before};

impl Config {
    /// Apply ONLY the allowlisted keys from a parsed overrides file, on top of
    /// the already-layered config (so overrides beat CLI). Records the names of
    /// fields actually changed (`overridden_fields`) and any known-but-not-
    /// overridable keys present in the file (`rejected_override_keys`) for a
    /// post-init warning. Unknown/typo keys are silently dropped by serde.
    pub(crate) fn apply_overrides(&mut self, file: &ConfigFile) {
        // ONLY allowlisted keys may be overridden at runtime. `apply_*` sets the
        // field and records its name; `reject` flags a known-but-not-overridable key
        // present in the file. Every config field belongs in exactly one list — a
        // field in neither is silently dropped by serde, so keep both exhaustive.
        let mut changed: Vec<&'static str> = Vec::new();
        macro_rules! apply_copy {
            ($src:expr => $dst:ident) => {
                if let Some(v) = $src {
                    self.$dst = v;
                    changed.push(stringify!($dst));
                }
            };
        }
        macro_rules! apply_secs {
            ($src:expr => $dst:ident) => {
                if let Some(v) = $src {
                    self.$dst = Duration::from_secs(v);
                    changed.push(stringify!($dst));
                }
            };
        }

        apply_secs!(file.subscription.fallback_poll_interval => subscription_fallback_poll_interval);
        apply_secs!(file.subscription.poll_wake_interval => subscription_poll_wake_interval);
        apply_copy!(file.subscription.fallback_max_episodes => subscription_fallback_max_episodes);
        apply_copy!(file.subscription.max_concurrent_downloads => subscription_max_concurrent_downloads);
        apply_copy!(file.subscription.max_poll_concurrent => subscription_max_poll_concurrent);
        apply_copy!(file.subscription.poll_auto_download_enabled => subscription_poll_auto_download_enabled);
        apply_copy!(file.subscription.auto_playlist_add_to_start => subscription_auto_playlist_add_to_start);
        if let Some(v) = &file.subscription.no_sync_before
            && let Some(d) = parse_no_sync_before(v)
        {
            self.subscription_no_sync_before = d;
            changed.push("subscription_no_sync_before");
        }
        apply_copy!(file.subscription.sync_on_start => subscription_sync_on_start);
        apply_copy!(file.auth.token_expiry_minutes => auth_token_expiry_minutes);
        apply_copy!(file.episode.playback_complete_percentage => episode_playback_complete_percentage);
        if let Some(v) = &file.opml.file {
            self.opml_file = Some(PathBuf::from(v));
            changed.push("opml_file");
        }

        self.overridden_fields = changed.into_iter().map(String::from).collect();

        // Keys present in the file but NOT on the allowlist (secrets, identity /
        // run-once fields, and the override knobs themselves) are ignored —
        // recorded here so `main` can warn once logging is up.
        let mut rejected: Vec<&'static str> = Vec::new();
        macro_rules! reject {
            ($src:expr, $key:literal) => {
                if $src.is_some() {
                    rejected.push($key);
                }
            };
        }
        reject!(file.auth.token_secret, "auth.token_secret");
        reject!(file.admin.username, "admin.username");
        reject!(file.admin.password, "admin.password");
        reject!(file.admin.disable_seed, "admin.disable_seed");
        reject!(file.server.listen_address, "server.listen_address");
        reject!(file.server.listen_port, "server.listen_port");
        reject!(
            file.server.disable_polling_service,
            "server.disable_polling_service"
        );
        reject!(
            file.server.cors_allowed_origins,
            "server.cors_allowed_origins"
        );
        reject!(file.db.path, "db.path");
        reject!(file.db.no_migrate, "db.no_migrate");
        reject!(file.db.no_wal, "db.no_wal");
        reject!(file.db.skip_default_playlist, "db.skip_default_playlist");
        reject!(file.media.root, "media.root");
        reject!(file.public_files.enable, "public.enable");
        reject!(file.public_files.root, "public.root");
        reject!(file.public_files.url_path, "public.url_path");
        reject!(file.log.file, "log.file");
        reject!(file.log.level, "log.level");
        reject!(file.log.target, "log.target");
        reject!(file.log.file_name, "log.file_name");
        reject!(file.log.line_number, "log.line_number");
        // Watchdog knobs are boot config (CLI/env/TOML), not runtime-overridable —
        // grouped with the other operational subscription/dev boot-only knobs.
        reject!(
            file.subscription.download_stuck_after,
            "subscription.download_stuck_after"
        );
        reject!(
            file.subscription.download_max_attempts,
            "subscription.download_max_attempts"
        );
        reject!(
            file.subscription.dev_use_mock_download,
            "subscription.dev_use_mock_download"
        );
        reject!(
            file.subscription.dev_seed_data,
            "subscription.dev_seed_data"
        );
        reject!(
            file.server.allow_private_network,
            "server.allow_private_network"
        );
        reject!(file.config_overrides.disable, "config_overrides.disable");
        reject!(file.config_overrides.path, "config_overrides.path");

        self.rejected_override_keys = rejected.into_iter().map(String::from).collect();
    }

    /// Load + apply the allowlisted overrides file at `path` onto an
    /// already-built config — the in-process (embedded server) equivalent of
    /// the overrides step in [`Config::resolve`], which is unreachable there
    /// because the embedded host builds its `Config` as a struct literal
    /// rather than via CLI/env layering. Records the same outcome fields
    /// (`overridden_fields` / `rejected_override_keys` /
    /// `config_overrides_loaded_from` / `config_overrides_load_error`) and
    /// pins `config_overrides_path` so the `/config-overrides` endpoints
    /// read/write the same file. A missing file is fine (nothing to apply).
    pub fn load_and_apply_overrides(&mut self, path: &Path) {
        self.config_overrides_path = Some(path.to_path_buf());
        if !path.exists() {
            return;
        }
        match ConfigFile::from_path(path) {
            Ok(file) => {
                self.apply_overrides(&file);
                self.config_overrides_loaded_from = Some(path.to_path_buf());
            }
            Err(e) => self.config_overrides_load_error = Some(e),
        }
    }
}

/// Resolve the effective overrides-file path: an explicit configured path wins;
/// otherwise it sits beside `--config` (if given); otherwise in the config dir
/// where `halogen.toml` would live. `None` only if no config dir is resolvable.
pub(crate) fn resolve_overrides_path(cli: &Cli, configured: Option<&Path>) -> Option<PathBuf> {
    if let Some(p) = configured {
        return Some(p.to_path_buf());
    }
    if let Some(cfg_path) = &cli.config {
        let dir = cfg_path.parent().unwrap_or_else(|| Path::new("."));
        return Some(dir.join(DEFAULT_CONFIG_OVERRIDES_FILENAME));
    }
    get_xdg_config_path()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .map(|dir| dir.join(DEFAULT_CONFIG_OVERRIDES_FILENAME))
}

/// Read the overrides file at `path` into the API DTO. A missing file yields an
/// empty (all-`None`) set; only a read/parse failure is an error. Only
/// allowlisted keys are surfaced — anything else in the file is dropped.
pub fn read_overrides(path: &Path) -> Result<ConfigOverridesData, String> {
    if !path.exists() {
        return Ok(ConfigOverridesData::default());
    }
    let file = ConfigFile::from_path(path)?;
    Ok(ConfigOverridesData {
        subscription_fallback_poll_interval_secs: file.subscription.fallback_poll_interval,
        subscription_poll_wake_interval_secs: file.subscription.poll_wake_interval,
        subscription_fallback_max_episodes: file.subscription.fallback_max_episodes,
        subscription_max_concurrent_downloads: file.subscription.max_concurrent_downloads,
        subscription_max_poll_concurrent: file.subscription.max_poll_concurrent,
        subscription_poll_auto_download_enabled: file.subscription.poll_auto_download_enabled,
        subscription_auto_playlist_add_to_start: file.subscription.auto_playlist_add_to_start,
        subscription_no_sync_before: file.subscription.no_sync_before.clone(),
        subscription_sync_on_start: file.subscription.sync_on_start,
        auth_token_expiry_minutes: file.auth.token_expiry_minutes,
        episode_playback_complete_percentage: file.episode.playback_complete_percentage,
        opml_file: file.opml.file.clone(),
    })
}

/// Serialize `overrides` to the sectioned TOML schema `ConfigFile` reads back,
/// then write it atomically (temp sibling + rename) so a concurrent boot never
/// reads a torn file. Creates the parent directory if needed.
pub fn write_overrides(path: &Path, overrides: &ConfigOverridesData) -> Result<(), String> {
    let out = OverridesToml::from(overrides);
    let body = toml::to_string(&out).map_err(|e| format!("Failed to serialize overrides: {e}"))?;

    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create overrides directory: {e}"))?;
    }

    let tmp = path.with_extension("toml.tmp");
    fs::write(&tmp, body.as_bytes()).map_err(|e| format!("Failed to write overrides file: {e}"))?;
    fs::rename(&tmp, path).map_err(|e| format!("Failed to persist overrides file: {e}"))?;
    Ok(())
}

// Serialization-only mirror of the allowlisted `ConfigFile` sections. `None`
// fields are skipped, and empty sections are omitted, so the persisted file
// lists only what's actually overridden.
#[derive(serde::Serialize, Default)]
struct OverridesToml {
    #[serde(skip_serializing_if = "OverridesSubscription::is_empty")]
    subscription: OverridesSubscription,
    #[serde(skip_serializing_if = "OverridesAuth::is_empty")]
    auth: OverridesAuth,
    #[serde(skip_serializing_if = "OverridesEpisode::is_empty")]
    episode: OverridesEpisode,
    #[serde(skip_serializing_if = "OverridesOpml::is_empty")]
    opml: OverridesOpml,
}

#[derive(serde::Serialize, Default)]
struct OverridesSubscription {
    #[serde(skip_serializing_if = "Option::is_none")]
    fallback_poll_interval: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    poll_wake_interval: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fallback_max_episodes: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_concurrent_downloads: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_poll_concurrent: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    poll_auto_download_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    auto_playlist_add_to_start: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    no_sync_before: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sync_on_start: Option<bool>,
}

#[derive(serde::Serialize, Default)]
struct OverridesAuth {
    #[serde(skip_serializing_if = "Option::is_none")]
    token_expiry_minutes: Option<u64>,
}

#[derive(serde::Serialize, Default)]
struct OverridesEpisode {
    #[serde(skip_serializing_if = "Option::is_none")]
    playback_complete_percentage: Option<u16>,
}

#[derive(serde::Serialize, Default)]
struct OverridesOpml {
    #[serde(skip_serializing_if = "Option::is_none")]
    file: Option<String>,
}

impl OverridesSubscription {
    fn is_empty(&self) -> bool {
        self.fallback_poll_interval.is_none()
            && self.poll_wake_interval.is_none()
            && self.fallback_max_episodes.is_none()
            && self.max_concurrent_downloads.is_none()
            && self.max_poll_concurrent.is_none()
            && self.poll_auto_download_enabled.is_none()
            && self.auto_playlist_add_to_start.is_none()
            && self.no_sync_before.is_none()
            && self.sync_on_start.is_none()
    }
}
impl OverridesAuth {
    fn is_empty(&self) -> bool {
        self.token_expiry_minutes.is_none()
    }
}
impl OverridesEpisode {
    fn is_empty(&self) -> bool {
        self.playback_complete_percentage.is_none()
    }
}
impl OverridesOpml {
    fn is_empty(&self) -> bool {
        self.file.is_none()
    }
}

impl From<&ConfigOverridesData> for OverridesToml {
    fn from(o: &ConfigOverridesData) -> Self {
        OverridesToml {
            subscription: OverridesSubscription {
                fallback_poll_interval: o.subscription_fallback_poll_interval_secs,
                poll_wake_interval: o.subscription_poll_wake_interval_secs,
                fallback_max_episodes: o.subscription_fallback_max_episodes,
                max_concurrent_downloads: o.subscription_max_concurrent_downloads,
                max_poll_concurrent: o.subscription_max_poll_concurrent,
                poll_auto_download_enabled: o.subscription_poll_auto_download_enabled,
                auto_playlist_add_to_start: o.subscription_auto_playlist_add_to_start,
                no_sync_before: o.subscription_no_sync_before.clone(),
                sync_on_start: o.subscription_sync_on_start,
            },
            auth: OverridesAuth {
                token_expiry_minutes: o.auth_token_expiry_minutes,
            },
            episode: OverridesEpisode {
                playback_complete_percentage: o.episode_playback_complete_percentage,
            },
            opml: OverridesOpml {
                file: o.opml_file.clone(),
            },
        }
    }
}
