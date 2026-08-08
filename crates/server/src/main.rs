use std::process;

use halogen_config::Config;
use halogen_migrate::{JournalMode, connect_and_migrate_with};
use halogen_server::restart::RestartHandle;
use sea_orm::EntityTrait;
use tracing::{error, info, warn};

/// What the server loop should do once `axum::serve` returns.
#[derive(PartialEq, Eq)]
enum Outcome {
    /// Normal shutdown (Ctrl-C / fatal) — exit the process.
    Shutdown,
    /// A restart was requested via the API — re-exec the process so it re-reads
    /// config (picking up a freshly written overrides file).
    Restart,
}

fn main() {
    // `run` owns the tokio runtime; it returns only after the server stops. Doing
    // the re-exec out here means the runtime is fully torn down first — no live
    // tokio threads when we replace the process image.
    match run() {
        Outcome::Restart => re_exec(),
        Outcome::Shutdown => {}
    }
}

#[tokio::main]
async fn run() -> Outcome {
    let cfg = halogen_config::build().unwrap_or_else(|e| {
        error!("Config error: {e}");
        process::exit(1);
    });
    halogen_server::logging::init(&cfg);
    info!("{cfg}");

    // SSRF guard: block outbound fetches (feeds, art, episode media) to non-public
    // hosts unless `allow_private_network` is set (CLI/env/TOML) — for dev, or a feed
    // host on your LAN. Blocked by default in production.
    halogen_net::configure(cfg.allow_private_network);
    // Outbound User-Agent: `server_fetch_user_agent` when set, else each client's
    // purpose default. Read when each client is first built, so it must land
    // before the first fetch.
    halogen_net::configure_user_agent(cfg.server_fetch_user_agent.clone());

    // Config overrides are loaded before logging exists (in `Config::resolve`),
    // so what happened is recorded on `cfg` and reported now.
    log_config_overrides(&cfg);

    let run_migrations = !cfg.db_no_migrate;
    // WAL by default: readers don't block the poll tick's writes, and
    // replication tooling requires it. `--db-no-wal` pins the rollback
    // journal instead (network-filesystem storage) — explicitly, because WAL
    // is sticky on the file and must be actively converted back.
    let journal = if cfg.db_no_wal {
        JournalMode::Delete
    } else {
        JournalMode::Wal
    };
    let dbc = connect_and_migrate_with(&cfg.db_path, run_migrations, journal)
        .await
        .unwrap_or_else(|e| {
            error!("Database error: {e}");
            process::exit(1);
        });

    seed_all(&dbc, &cfg).await;
    run_startup_opml(&dbc, &cfg).await;

    // Reset downloads orphaned by a previous run (crash / pod roll mid-fetch):
    // flip every stuck `Downloading` row to `DownloadError` so the poller's
    // recovery pass re-attempts them. Runs before any startup sync can download.
    match halogen_download::reclaim_orphaned_downloads(&dbc).await {
        Ok(n) if n > 0 => info!("Reset {n} orphaned download(s) on startup"),
        Ok(_) => {}
        Err(e) => tracing::warn!("Failed to reclaim orphaned downloads on startup: {e}"),
    }

    let polling = halogen_polling::PollingHandle::from_config(dbc.clone(), &cfg);

    // A start-up sync runs through the handle (force every feed, honour
    // auto-download + retention) — not the legacy bare sync.
    if cfg.subscription_sync_on_start {
        match polling.poll().await {
            Ok(()) => {
                info!("\nSubscription sync completed successfully.");
            }
            Err(e) => {
                error!("Subscription sync error: {}", e);
                process::exit(1);
            }
        }
    }

    if let Err(e) = polling.start() {
        error!("Failed to start polling service: {}", e);
    }

    // Coordinator the restart endpoint trips; `main` re-execs when it's set.
    let restart = RestartHandle::new();
    let router = halogen_server::routers::build_router(dbc, &cfg, polling, restart.clone());

    let addr = format!("{}:{}", cfg.listen_address, cfg.listen_port);
    info!("Listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|e| {
            error!("Failed to bind to {addr}: {e}");
            process::exit(1);
        });

    // Graceful shutdown on either an API-requested restart or Ctrl-C; in-flight
    // requests (incl. the restart response itself) drain before serve returns.
    let shutdown = {
        let restart = restart.clone();
        async move {
            let ctrl_c = async {
                let _ = tokio::signal::ctrl_c().await;
            };
            tokio::select! {
                _ = restart.wait() => {},
                _ = ctrl_c => {},
            }
        }
    };

    if let Err(e) = axum::serve(listener, router)
        .with_graceful_shutdown(shutdown)
        .await
    {
        error!("Server exited with a fatal error: {e}");
        process::exit(1);
    }

    if restart.is_requested() {
        info!("Restart requested — re-executing process");
        Outcome::Restart
    } else {
        Outcome::Shutdown
    }
}

/// Idempotent start-up seeding: the admin user, the default "Queue" playlist, and
/// (debug builds only) dev fixtures. A failed REQUIRED seed `process::exit(1)`s;
/// the optional ones log and continue — the exit-vs-continue policy is per-step on
/// purpose, and now visible in one place.
async fn seed_all(dbc: &sea_orm::DatabaseConnection, cfg: &Config) {
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
async fn run_startup_opml(dbc: &sea_orm::DatabaseConnection, cfg: &Config) {
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
fn log_config_overrides(cfg: &Config) {
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

/// Replace the current process image with a fresh invocation of this binary
/// (same argv + inherited env), so the new process re-runs `Config::resolve` and
/// applies the (possibly just-written) overrides file. `execv` only returns on
/// failure.
#[cfg(unix)]
fn re_exec() -> ! {
    use std::os::unix::process::CommandExt;
    let exe = std::env::current_exe().unwrap_or_else(|e| {
        error!("re-exec: cannot determine current executable: {e}");
        process::exit(1);
    });
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    let err = std::process::Command::new(exe).args(args).exec();
    error!("re-exec failed: {err}");
    process::exit(1);
}

#[cfg(not(unix))]
fn re_exec() -> ! {
    error!("Restart via re-exec is only supported on Unix; exiting instead");
    process::exit(0);
}
