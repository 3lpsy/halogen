use axum::{
    Extension, Router,
    routing::{delete as route_delete, get as route_get, post as route_post, put as route_put},
};
use halogen_routes::routers::*;
use tower_http::compression::CompressionLayer;

/// Frontend CSP restricts network egress to this server; it is not an XSS policy. Allow blob audio and data images,
/// inline scripts/styles for Dioxus, eval/WASM for bootstrap and document::eval, and same-origin workers for sync. Art
/// and audio must use server caches or local blobs.
pub const FRONTEND_CSP: &str = "default-src 'self'; \
     img-src 'self' data:; \
     media-src 'self' blob:; \
     connect-src 'self'; \
     script-src 'self' 'unsafe-inline' 'unsafe-eval' 'wasm-unsafe-eval'; \
     worker-src 'self'; \
     style-src 'self' 'unsafe-inline'; \
     font-src 'self' data:; \
     object-src 'none'; \
     base-uri 'self'";

use halogen_polling::PollingHandle;
use halogen_routes::auth::{change_password, login, logout, refresh};
use halogen_routes::middleware::AuthConfig;
use halogen_routes::middleware::JwtAuthLayer;
use halogen_routes::polling::{
    AppState, get_poll_job, list_poll_jobs, poll, start, start_poll_job, status, stop,
};
use halogen_routes::routers::restart::request_restart;
use halogen_runtime_control::RestartHandle;

/// Path prefix mounting every HTTP API, keeping them clear of SPA routes.
pub const API_PREFIX: &str = "/api/v1";

pub fn build_router(
    dbc: sea_orm::DatabaseConnection,
    cfg: &halogen_config::Config,
    polling: PollingHandle,
    restart: RestartHandle,
) -> Router<()> {
    register(dbc, cfg, polling, restart, None)
}

/// Dispatch local requests as the actor selected by the library session.
pub fn router_local(
    dbc: sea_orm::DatabaseConnection,
    cfg: &halogen_config::Config,
    polling: PollingHandle,
    restart: RestartHandle,
    actor: middleware::JwtAuth,
) -> Router {
    register(dbc, cfg, polling, restart, Some(actor))
}

fn register(
    dbc: sea_orm::DatabaseConnection,
    cfg: &halogen_config::Config,
    polling: PollingHandle,
    restart: RestartHandle,
    local_actor: Option<middleware::JwtAuth>,
) -> Router {
    // One auth config: the layer below validates bearer tokens with it, and the
    // login/refresh/media handlers read the same struct as an `Extension`.
    let auth_config = AuthConfig {
        secret: cfg.auth_token_secret.clone(),
        // Saturating: `Config::resolve` already clamps the minutes to a sane
        // range, but never overflow here regardless of how this cfg was built.
        expiry_secs: cfg.auth_token_expiry_minutes.saturating_mul(60),
    };

    // Source the shared download tracker before `polling` is moved into the app
    // state, so the manual download path (`MediaDownloadConfig`) and the progress
    // API (`state.polling.download_tracker()`) share the one tracker.
    let download_tracker = polling.download_tracker();
    let app_state = AppState { polling };

    // Auth routes — public, EXCEPT `/password`, which changes the caller's own
    // password and so needs authentication. The surrounding routes are public, so
    // rather than splitting the nest we give just `/password` its own JWT layer;
    // the user id then comes from the token (`AuthUserId`), never the body.
    let auth_routes = Router::new()
        .route("/login", route_post(login))
        .route("/logout", route_post(logout))
        .route("/refresh", route_post(refresh))
        .route(
            "/password",
            route_post(change_password).layer(JwtAuthLayer::new(dbc.clone(), auth_config.clone())),
        );

    let public_routes = Router::new()
        .nest("/auth", auth_routes)
        // Build-version probe — public like `/healthz`, but inside `/api/v1`
        // so it reports on the API deployment specifically.
        .route("/version", route_get(health::version));

    // Media routes authenticate through media_auth outside bearer middleware because img/audio elements cannot set
    // headers. Accept scoped media cookies or API bearer tokens; fetch origin art server-side and enforce client egress
    // through CSP.
    let media_routes = Router::new()
        .route("/episodes/{id}/audio", route_get(episodes::audio))
        .route("/episodes/{id}/art", route_get(episodes::art))
        .route("/episodes/{id}/art/small", route_get(episodes::art_small))
        .route("/podcasts/{id}/art", route_get(podcasts::art))
        .route("/podcasts/{id}/art/small", route_get(podcasts::art_small));

    // Connectivity WebSocket — outside the bearer middleware for the same reason
    // as the media routes (a browser WebSocket handshake can't send an
    // Authorization header). It authenticates from the `?ticket=` query param (a
    // short-lived WS-scoped JWT minted by the protected `POST /ws-ticket` below).
    let ws_routes = Router::new().route("/ws", route_get(ws::ws));

    // Strictly ADMIN-ONLY operations, nested under `/api/v1/admin`. The handlers all enforce `AdminUser` (403
    // for non-admins) — the prefix makes the admin surface explicit in the URL space. Owner-or-admin routes
    // (e.g. `PUT /podcasts/{id}`, `GET/PUT /users/{id}`) are NOT admin-only and stay unprefixed below.
    let admin_routes = Router::new()
        // User management: list + delete are admin-only (get/update are
        // self-or-admin and live unprefixed).
        .route("/users", route_get(users::list))
        .route("/users/{id}", route_delete(users::delete))
        // Polling controls (`/status` is a plain-auth read and stays unprefixed).
        .route("/poll", route_post(poll))
        // On-demand poll jobs: start one (optionally `?podcast_id=`), poll its
        // status, list recent history.
        .route("/poll-job", route_post(start_poll_job))
        .route("/poll-job/{id}", route_get(get_poll_job))
        .route("/poll-jobs", route_get(list_poll_jobs))
        .route("/start", route_post(start))
        .route("/stop", route_post(stop))
        // Reconciled runtime config (secrets omitted).
        .route("/config", route_get(config::get))
        // Writable overrides allowlist; none restart (apply via POST
        // /server/restart). GET reads the persisted set (prepopulates the editor),
        // POST replaces it wholesale (omitting a key deletes that override),
        // DELETE clears all.
        .route("/config-overrides", route_get(config::get_overrides))
        .route("/config-overrides", route_post(config::set_overrides))
        .route("/config-overrides", route_delete(config::delete_overrides))
        // Request a graceful restart (re-exec) to apply overrides.
        .route("/server/restart", route_post(request_restart))
        // Tail of the server's log file.
        .route("/server-logs", route_get(server_logs::get))
        // Persisted failure histories (RSS sync + episode downloads).
        .route("/server-errors", route_get(server_errors::get))
        // OPML import/export.
        .route("/opml/import", route_post(opml::import::import))
        .route("/opml/export", route_get(opml::export::export))
        // User creation (list/delete above; get/update stay unprefixed).
        .route("/users", route_post(users::store))
        // Whole-database export/import: the server↔server / embedded↔server
        // migration + full-backup story. The import route carries its own
        // body-size cap (a library DB outgrows the 2 MB default).
        .route("/db/export", route_get(db_transfer::export))
        .route(
            "/db/import",
            route_post(db_transfer::import).layer(axum::extract::DefaultBodyLimit::max(
                db_transfer::DB_IMPORT_MAX_BYTES,
            )),
        );

    // Protected routes — JWT auth required
    let protected_routes = Router::new()
        .route("/sync/changes", route_get(sync::changes))
        .nest("/admin", admin_routes)
        .route("/users/{id}", route_get(users::get))
        .route("/users/{id}", route_put(users::update))
        .route("/podcasts", route_get(podcasts::list))
        .route("/podcasts", route_post(podcasts::store))
        .route("/podcasts/{id}", route_get(podcasts::get))
        .route(
            "/podcasts/{id}/episodes",
            route_get(podcasts::episodes_list),
        )
        // Create + link / unlink + delete a podcast's download/poll config
        // atomically (editing an existing config uses PUT /podcast-configs/{id}).
        .route("/podcasts/{id}/config", route_post(podcasts::config_store))
        .route(
            "/podcasts/{id}/config",
            route_delete(podcasts::config_delete),
        )
        // Read / replace the playlists a podcast auto-adds new episodes to.
        .route(
            "/podcasts/{id}/auto-playlists",
            route_get(podcasts::auto_playlists_get),
        )
        .route(
            "/podcasts/{id}/auto-playlists",
            route_put(podcasts::auto_playlists_set),
        )
        .route("/podcasts/{id}", route_put(podcasts::update))
        .route("/podcasts/{id}", route_delete(podcasts::delete))
        .route("/episodes", route_get(episodes::list))
        .route("/episodes", route_post(episodes::store))
        .route(
            "/episodes/{id}/download",
            route_post(episodes::download).delete(episodes::remove),
        )
        // Bulk variants: one request acts on many episode ids (authorized ones are
        // kept, the rest filtered out). `download/bulk` (trailing `bulk`) avoids
        // colliding with `/episodes/{id}/download`.
        .route(
            "/episodes/download/bulk",
            route_post(episodes::download_bulk).delete(episodes::remove_bulk),
        )
        // Live in-flight download progress: the static `download-progress` list
        // lives beside the `download/bulk` literal so it isn't shadowed by the
        // `/episodes/{id}` param route.
        .route(
            "/episodes/download-progress",
            route_get(episodes::download_progress_active),
        )
        .route(
            "/episodes/{id}/download-progress",
            route_get(episodes::download_progress),
        )
        .route("/episodes/{id}", route_get(episodes::get))
        .route("/episodes/{id}", route_put(episodes::update))
        .route("/episodes/{id}", route_delete(episodes::delete))
        // Which of the caller's playlists this episode is in (picker pre-selection).
        .route(
            "/episodes/{id}/playlists",
            route_get(episodes::playlists_list),
        )
        // Standalone configs: only read (owner-or-admin) + edit (owner-or-admin)
        // are exposed. Configs are CREATED + DELETED through the owning podcast
        // (`POST`/`DELETE /podcasts/{id}/config`); there is no unlinked create,
        // list, or delete (the UI never used them).
        .route("/podcast-configs/{id}", route_get(podcast_configs::get))
        .route("/podcast-configs/{id}", route_put(podcast_configs::update))
        .route("/playlists", route_get(playlists::list))
        .route("/playlists", route_post(playlists::store))
        .route("/playlists/default", route_get(playlists::get_default))
        .route("/playlists/{id}", route_get(playlists::get))
        .route("/playlists/{id}", route_put(playlists::update))
        .route("/playlists/{id}", route_delete(playlists::delete))
        // Reorder a playlist within the user's manual (position) order.
        .route("/playlists/{id}/move", route_post(playlists::move_playlist))
        // Smart-reorder a playlist's episodes by a chosen field + direction.
        .route(
            "/playlists/{id}/reorder-by",
            route_post(playlists::reorder_by),
        )
        .route(
            "/playlists/{id}/episodes/{episode_id}",
            route_post(playlists::store_episode_playlist),
        )
        .route(
            "/playlists/{id}/episodes/{episode_id}",
            route_delete(playlists::delete_episode_playlist),
        )
        .route(
            "/playlists/{id}/episodes/{episode_id}/move",
            route_post(playlists::move_episode_playlist),
        )
        .route(
            "/playlists/{id}/episodes/bulk",
            route_post(playlists::bulk_store_episode_playlist)
                .delete(playlists::bulk_delete_episode_playlist),
        )
        .route(
            "/playlists/{id}/episodes",
            route_get(playlists::episodes_list),
        )
        .route("/playbacks", route_get(playbacks::list))
        .route("/playbacks", route_post(playbacks::store))
        .route("/playbacks/{id}", route_get(playbacks::get))
        .route("/playbacks/{id}", route_delete(playbacks::delete))
        // Polling status: a read any authed user may do (the admin-only control
        // operations live under `/admin` above).
        .route("/status", route_get(status))
        // Mint a short-lived ticket for the `/ws` connectivity socket (the socket
        // itself is mounted unauthed-by-middleware in `ws_routes`).
        .route("/ws-ticket", route_post(ws::ticket))
        // Online-only podcast discovery (federated provider search). Any authed
        // user; the DiscoverService Extension is layered on `api` below.
        .route("/discover/search", route_get(discover::search::search))
        .route(
            "/discover/search/page",
            route_get(discover::search::podcast_page),
        )
        .route(
            "/discover/episodes/search/page",
            route_get(discover::search::episode_page),
        )
        .route(
            "/discover/episodes/search",
            route_get(discover::search::episodes),
        )
        .route("/discover/podcast", route_get(discover::search::podcast))
        .route(
            "/discover/providers",
            route_get(discover::providers::providers),
        )
        // Compress protected JSON responses only. Keep range-served audio, already-compressed art, and WebSocket
        // upgrades outside this layer; CompressionLayer negotiates encoding and skips unsuitable bodies.
        .layer(CompressionLayer::new());

    let is_local = local_actor.is_some();
    let protected_routes = match local_actor {
        Some(actor) => protected_routes.layer(axum::middleware::from_fn_with_state(
            (dbc.clone(), actor.user_id.parse::<i32>().unwrap_or(0)),
            halogen_auth::local_actor,
        )),
        None => protected_routes.layer(JwtAuthLayer::new(dbc.clone(), auth_config.clone())),
    };
    let network_routes = if is_local {
        Router::new()
    } else {
        Router::new()
            .merge(public_routes)
            .merge(media_routes)
            .merge(ws_routes)
    };

    // Knobs the on-demand download handler needs (where to write, mock vs real).
    let media_download = episodes::MediaDownloadConfig {
        media_root: cfg.media_root.clone(),
        use_mock_download: cfg.dev_use_mock_download,
        fallback_max_episodes: cfg.subscription_fallback_max_episodes,
        tracker: download_tracker,
    };

    // All HTTP APIs live under `/api/v1` so no API path can shadow a client-side
    // SPA route. A browser refresh of e.g. `/podcasts` (no Authorization header)
    // must fall through to the SPA `index.html` fallback, not hit a guarded API
    // route — moving the API under a prefix guarantees that.
    let api = Router::new()
        .merge(network_routes)
        .merge(protected_routes)
        // Enable form decoding before bracket parsing so URL-encoded includes%5B0%5D and raw includes[0] are
        // equivalent. Otherwise serde_qs silently loses URLSession-style array/filter parameters.
        .layer(Extension(
            serde_qs::axum::QsQueryConfig::new()
                .config(serde_qs::Config::new().use_form_encoding(true)),
        ))
        .layer(Extension(auth_config))
        .layer(Extension(dbc.clone()))
        .layer(Extension(app_state))
        .layer(Extension(media_download))
        .layer(Extension(
            halogen_handlers::playback::PlaybackCompleteConfig(
                cfg.episode_playback_complete_percentage,
            ),
        ))
        // Sanitised snapshot of the reconciled config for the admin `GET /config`.
        .layer(Extension(config::config_data(cfg)))
        // Log-file location for the admin `GET /admin/server-logs` tail.
        .layer(Extension(server_logs::ServerLogsConfig {
            log_file: cfg.log_file.clone(),
        }))
        // Online-only podcast discovery service (one shared reqwest client).
        // Provider base URLs come from config (real hosts by default; tests
        // point them at a wiremock server).
        .layer(Extension(std::sync::Arc::new(
            halogen_discover::DiscoverService::with_bases(
                cfg.discover_itunes_base_url.clone(),
                cfg.discover_gpodder_base_url.clone(),
            ),
        )))
        // Restart coordinator for the admin `POST /server/restart`.
        .layer(Extension(restart));

    Router::new()
        .route("/healthz", route_get(health::healthz))
        .nest(API_PREFIX, api)
}
