use halogen_config::Config;
use sea_orm::EntityTrait;
use std::process;
use tracing::{error, info, warn};

/// Idempotent start-up seeding: the admin user, the default "Queue" playlist, and
/// (debug builds only) dev fixtures. A failed REQUIRED seed `process::exit(1)`s;
/// the optional ones log and continue — the exit-vs-continue policy is per-step on
/// purpose, and now visible in one place.
pub(crate) async fn seed_all(dbc: &sea_orm::DatabaseConnection, cfg: &Config) {
    if let Some(username) = &cfg.admin_username
        && !cfg.admin_disable_seed
        && !username.is_empty()
    {
        let password = cfg.admin_password.as_deref();
        if let Err(e) = halogen_fixture::user::seed_admin_user(dbc, username, password).await {
            error!("Admin seed error: {e}");
            process::exit(1);
        }
    }

    // Ensure the default "Queue" playlist exists (idempotent) unless disabled.
    if !cfg.db_skip_default_playlist {
        match halogen_fixture::playlist::seed_default_queue(dbc).await {
            Ok(true) => info!("Default \"Queue\" playlist created"),
            Ok(false) => {}
            Err(e) => error!("Default playlist seed error: {e}"),
        }
    }

    // Dev data seeding vanishes from release builds unless the `dev-seed`
    // feature opts in (the e2e harness server — see the fixture crate docs).
    #[cfg(any(debug_assertions, feature = "dev-seed"))]
    if cfg.dev_seed_data {
        // Fixtures (RSS feeds + nasa-test-clip.mp3) live at the repo's data/tests; the dev
        // server runs from the repo root (see the `dev-server` just recipe). The
        // seed is idempotent — it no-ops if podcasts already exist.
        let fixtures_dir = std::path::PathBuf::from("data/tests");
        match halogen_fixture::dev::seed_dev_data(dbc, &fixtures_dir, &cfg.media_root).await {
            Ok(true) => info!("Dev data seeded"),
            Ok(false) => info!("Dev data already present — skipping seed"),
            Err(e) => {
                error!("Dev data seed error: {e}");
                process::exit(1);
            }
        }

        // A second, non-admin account (`dev2` / `dev2`) with a small library, so
        // the multi-user / subscription-scoping paths are visible in dev. Secondary
        // to the main seed — log on failure rather than exit.
        match halogen_fixture::dev::seed_dev2_data(dbc).await {
            Ok(true) => info!("Dev2 (non-admin) user seeded"),
            Ok(false) => {}
            Err(e) => error!("Dev2 seed error: {e}"),
        }
    }
}

/// Import podcasts from the configured OPML file (if any), attributing them to the
/// first user (the seeded admin). A missing file, user-lookup failure, or import
/// failure is fatal; no-ops when no OPML is configured or no user exists yet.
pub(crate) async fn run_startup_opml(dbc: &sea_orm::DatabaseConnection, cfg: &Config) {
    let Some(opml_path) = &cfg.opml_file else {
        return;
    };
    if !opml_path.exists() {
        error!("OPML file not found: {}", opml_path.display());
        process::exit(1);
    }

    // Imported podcasts need an owner; attribute them to the first user.
    let owner = match halogen_orm::user::Entity::find().one(dbc).await {
        Ok(Some(owner)) => owner,
        Ok(None) => {
            warn!("No user present — skipping OPML import");
            return;
        }
        Err(e) => {
            error!("OPML import error (user lookup): {e}");
            process::exit(1);
        }
    };

    match halogen_opml::import_podcasts_from_opml(dbc, opml_path, owner.id).await {
        Ok(result) => {
            info!(
                "\nOPML import results:
  Total: {}
  Created: {}
  Skipped: {}
  Errors: {}",
                result.total, result.created, result.skipped, result.errors
            );
        }
        Err(e) => {
            error!("OPML import error: {}", e);
            process::exit(1);
        }
    }
}

/// Log the outcome of the pre-logging config-overrides load (recorded on `cfg`
/// during `Config::resolve`, before the subscriber existed).
pub(crate) fn log_config_overrides(cfg: &Config) {
    if let Some(path) = &cfg.config_overrides_loaded_from {
        if cfg.overridden_fields.is_empty() {
            info!(
                "Config overrides: loaded {} (no allowlisted keys set)",
                path.display()
            );
        } else {
            info!(
                "Config overrides from {}: {}",
                path.display(),
                cfg.overridden_fields.join(", ")
            );
        }
    }
    for key in &cfg.rejected_override_keys {
        warn!("Config overrides: ignoring non-overridable key `{key}`");
    }
    if let Some(e) = &cfg.config_overrides_load_error {
        warn!("Config overrides: failed to load ({e}); continuing without them");
    }
}
