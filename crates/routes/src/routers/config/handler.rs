//! Admin-only reconciled ConfigData omits JWT and admin-password secrets. Config-overrides GET reads, POST replaces the
//! allowlisted set, and DELETE clears it; omitted keys are removed. Writes take effect only after a separate server
//! restart.

use axum::{Extension, Json};
use halogen_utils::constants::{VALIDATION_CONFLICT_CODE, VALIDATION_PANIC_CODE};
use halogen_wire::{ConfigData, ConfigOverridesData, ResponseData};
use tracing::info;

use crate::routers::errors::ApiError;
use crate::routers::extractors::{AdminUser, Body};
use halogen_config::{self as config_svc, Config};

/// Project Config to ConfigData without auth_token_secret or admin_password; convert paths to strings and durations to
/// seconds. A free function avoids the foreign From/ConfigData orphan-rule restriction.
pub fn config_data(c: &Config) -> ConfigData {
    let path_str = |p: &std::path::Path| p.to_string_lossy().to_string();
    ConfigData {
        listen_address: c.listen_address.clone(),
        listen_port: c.listen_port,
        server_disable_polling_service: c.server_disable_polling_service,
        db_path: path_str(&c.db_path),
        db_no_migrate: c.db_no_migrate,
        db_skip_default_playlist: c.db_skip_default_playlist,
        media_root: path_str(&c.media_root),
        cors_allowed_origins: c.cors_allowed_origins.clone(),
        enable_public_server: c.enable_public_server,
        public_root: c.public_root.as_deref().map(path_str),
        public_url_path: c.public_url_path.clone(),
        subscription_fallback_poll_interval_secs: c.subscription_fallback_poll_interval.as_secs(),
        subscription_poll_wake_interval_secs: c.subscription_poll_wake_interval.as_secs(),
        subscription_fallback_max_episodes: c.subscription_fallback_max_episodes,
        subscription_max_concurrent_downloads: c.subscription_max_concurrent_downloads,
        subscription_max_poll_concurrent: c.subscription_max_poll_concurrent,
        subscription_poll_auto_download_enabled: c.subscription_poll_auto_download_enabled,
        subscription_auto_playlist_add_to_start: c.subscription_auto_playlist_add_to_start,
        subscription_no_sync_before: c.subscription_no_sync_before.to_string(),
        subscription_sync_on_start: c.subscription_sync_on_start,
        auth_token_expiry_minutes: c.auth_token_expiry_minutes,
        episode_playback_complete_percentage: c.episode_playback_complete_percentage,
        log_file: c.log_file.as_deref().map(path_str),
        log_level: c.log_level.clone(),
        log_target: c.log_target,
        log_file_name: c.log_file_name,
        log_line_number: c.log_line_number,
        admin_username: c.admin_username.clone(),
        admin_disable_seed: c.admin_disable_seed,
        opml_file: c.opml_file.as_deref().map(path_str),
        dev_use_mock_download: c.dev_use_mock_download,
        dev_seed_data: c.dev_seed_data,
        overridden_fields: c.overridden_fields.clone(),
        config_overrides_disabled: c.config_overrides_disable,
        config_overrides_path: c.config_overrides_path.as_deref().map(path_str),
        config_overrides_loaded: c.config_overrides_loaded_from.is_some(),
    }
}

/// GET /config — the reconciled runtime config, minus secrets. **Admin only.**
///
/// The sanitised [`ConfigData`] is built once at router construction and handed
/// in as an `Extension`, so this handler just echoes it through the envelope.
pub async fn get(
    Extension(config): Extension<ConfigData>,
    _admin: AdminUser,
) -> Json<ResponseData<ConfigData>> {
    Json(ResponseData::from_data(config))
}

/// GET /config-overrides — the raw, currently-persisted overrides (only the keys
/// actually overridden). **Admin only.** Empty when no overrides file (or path)
/// exists. The editor calls this to prepopulate its fields.
pub async fn get_overrides(
    Extension(config): Extension<ConfigData>,
    _admin: AdminUser,
) -> Result<Json<ResponseData<ConfigOverridesData>>, ApiError> {
    // No path resolved → nothing is overridden; return an empty set rather than
    // erroring (a clean "no overrides" answer for the editor).
    let current = match config.config_overrides_path.as_ref() {
        Some(p) => config_svc::read_overrides(&std::path::PathBuf::from(p))
            .map_err(|e| ApiError::new("config_overrides", VALIDATION_PANIC_CODE, e))?,
        None => ConfigOverridesData::default(),
    };
    Ok(Json(ResponseData::from_data(current)))
}

/// Admin-only replacement of config overrides; omitted keys are deleted. Body validates allowlisted fields and values.
/// Return 409 when disabled; changes apply only after a separate server restart.
pub async fn set_overrides(
    Extension(config): Extension<ConfigData>,
    _admin: AdminUser,
    Body(overrides): Body<ConfigOverridesData>,
) -> Result<Json<ResponseData<ConfigOverridesData>>, ApiError> {
    let path = writable_overrides_path(&config)?;
    config_svc::write_overrides(&path, &overrides)
        .map_err(|e| ApiError::new("config_overrides", VALIDATION_PANIC_CODE, e))?;
    info!(
        "Config overrides replaced at {} — restart required to apply",
        path.display()
    );
    Ok(Json(ResponseData::from_data(overrides)))
}

/// DELETE /config-overrides — clear ALL overrides (writes an empty file).
/// **Admin only.** `409` if overrides are disabled. Restart required to apply.
pub async fn delete_overrides(
    Extension(config): Extension<ConfigData>,
    _admin: AdminUser,
) -> Result<Json<ResponseData<()>>, ApiError> {
    let path = writable_overrides_path(&config)?;
    config_svc::write_overrides(&path, &ConfigOverridesData::default())
        .map_err(|e| ApiError::new("config_overrides", VALIDATION_PANIC_CODE, e))?;
    info!(
        "Config overrides cleared at {} — restart required to apply",
        path.display()
    );
    Ok(Json(ResponseData::from_data(())))
}

/// Resolve the overrides file path for a **mutating** request: `409` if the
/// override mechanism is disabled for this process, `500` if no path could be
/// resolved (real boots always resolve a default path even without a file).
fn writable_overrides_path(config: &ConfigData) -> Result<std::path::PathBuf, ApiError> {
    if config.config_overrides_disabled {
        return Err(ApiError::new(
            "config_overrides",
            VALIDATION_CONFLICT_CODE,
            "Config overrides are disabled for this server".to_string(),
        ));
    }
    config
        .config_overrides_path
        .as_ref()
        .map(std::path::PathBuf::from)
        .ok_or_else(|| {
            ApiError::new(
                "config_overrides",
                VALIDATION_PANIC_CODE,
                "No overrides path could be resolved".to_string(),
            )
        })
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
