//! Embedded `Config` construction — the "compatible configuration" the design
//! doc specifies (§6.3): a struct literal over `Config::default()` (never
//! `Config::resolve()`, which reads the *user's* XDG `halogen.toml` +
//! `HALOGEN_*` env — those belong to a real server install), with absolute
//! paths under the embedded root and the runtime overrides file layered on
//! top so the admin config UI keeps working.

use halogen_config::Config;
use tracing::{info, warn};

use crate::dirs::EmbeddedDirs;
use crate::secrets::Secrets;

pub(crate) fn build_config(dirs: &EmbeddedDirs, secrets: &Secrets) -> Config {
    let mut cfg = Config {
        auth_token_secret: secrets.auth_token_secret.clone(),
        // A year, not the server's week: weekly re-auth is pointless when the
        // password sits on the same disk as the token, and the UI's silent
        // re-login is the backstop either way.
        auth_token_expiry_minutes: 60 * 24 * 365,
        // Server defaults are cwd-relative ("halogen.db", "./media") —
        // unusable inside an app; everything lives under the embedded root.
        db_path: dirs.db_path(),
        media_root: dirs.media_root(),
        // The server default (2026-01-01) exists to keep the hosted
        // instance's first sync from ingesting years of back-catalog. A
        // standalone library starts empty and caps ingestion at
        // `fallback_max_episodes` per feed anyway — without this, a podcast
        // that stopped publishing before the cutoff would look permanently
        // empty. Runtime-overridable (subscription.no_sync_before).
        subscription_no_sync_before: chrono::NaiveDate::from_ymd_opt(1970, 1, 1)
            .expect("valid date"),
        ..Default::default()
    };
    // Layered last, like the overrides step in `Config::resolve` — also pins
    // `config_overrides_path` for the `/config-overrides` endpoints. An
    // in-process restart re-runs this, which is exactly how a freshly
    // PATCHed overrides file takes effect.
    cfg.load_and_apply_overrides(&dirs.overrides_path());
    cfg
}

/// The overrides-load outcome is recorded on `cfg` (this mirrors `main`'s
/// `log_config_overrides`; the fields exist because the binary loads overrides
/// before logging is up — here the host's subscriber is already live).
pub(crate) fn log_config_overrides(cfg: &Config) {
    if let Some(path) = &cfg.config_overrides_loaded_from {
        if cfg.overridden_fields.is_empty() {
            info!(
                "Embedded config overrides: loaded {} (no allowlisted keys set)",
                path.display()
            );
        } else {
            info!(
                "Embedded config overrides from {}: {}",
                path.display(),
                cfg.overridden_fields.join(", ")
            );
        }
    }
    for key in &cfg.rejected_override_keys {
        warn!("Embedded config overrides: ignoring non-overridable key `{key}`");
    }
    if let Some(e) = &cfg.config_overrides_load_error {
        warn!("Embedded config overrides: failed to load ({e}); continuing without them");
    }
}
