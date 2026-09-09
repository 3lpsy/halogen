use axum::Router;
use halogen_config::Config;
use halogen_polling::PollingHandle;
use halogen_runtime_control::RestartHandle;
use sea_orm::DatabaseConnection;

/// Add HTTP hosting concerns to the shared application router.
pub fn build_router(
    db: DatabaseConnection,
    cfg: &Config,
    polling: PollingHandle,
    restart: RestartHandle,
) -> Router {
    let mut app = halogen_router::build_router(db, cfg, polling, restart)
        .layer(crate::cors::build_cors(&cfg.cors_allowed_origins));
    if cfg.enable_public_server
        && let Some(root) = &cfg.public_root
    {
        app = crate::public::mount(app, &cfg.public_url_path, root);
    }
    #[cfg(feature = "embed-frontend")]
    if !cfg.enable_public_server {
        app = crate::embedded::mount(app);
    }
    app
}
