use std::process;

use halogen_migrations::{JournalMode, connect_and_migrate_with};
use halogen_server::restart::RestartHandle;
use tracing::{error, info};
mod lifecycle;
mod startup;
use lifecycle::re_exec;
use startup::{log_config_overrides, run_startup_opml, seed_all};

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
