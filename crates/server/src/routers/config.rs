//! Admin-only view of the reconciled runtime configuration.
//!
//! `GET /api/v1/config` returns the [`Config`](halogen_config::Config) actually in
//! effect after all sources are layered (defaults < TOML < env < CLI < overrides
//! file), wrapped in the standard `ResponseData` envelope. It is gated by the
//! [`AdminUser`] extractor (401 without a token, 403 for non-admins).
//!
//! The sibling `/config-overrides` resource (this module) is the writable
//! allowlist: `GET` reads the persisted overrides (to prepopulate the editor),
//! `POST` **replaces** them wholesale (so omitting a key deletes that override),
//! and `DELETE` clears them all. None restart — the new values apply only after
//! `POST /server/restart` re-execs the process.
//!
//! **Secrets are omitted.** [`ConfigData`] deliberately drops the fields that
//! would leak credentials — `auth_token_secret` (the JWT signing key / "api
//! token") and `admin_password` — so the endpoint is safe for an operator UI.

use axum::{Extension, Json};
use halogen_utils::constants::{VALIDATION_CONFLICT_CODE, VALIDATION_PANIC_CODE};
use halogen_wire::{ConfigData, ConfigOverridesData, ResponseData};
use tracing::info;

use crate::routers::errors::ApiError;
use crate::routers::extractors::{AdminUser, Body};
use halogen_config::{self as config_svc, Config};

/// Project the reconciled [`Config`] into the sanitised, shared [`ConfigData`].
///
/// Mirrors every runtime setting **except** the two secret fields
/// (`auth_token_secret`, `admin_password`), which are intentionally dropped so
/// they can never be serialised out. Paths are rendered as strings and
/// `Duration`s as whole seconds. (A free function rather than `From`: the orphan
/// rule forbids implementing the foreign `From` for the foreign `ConfigData`.)
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

/// POST /config-overrides — **replace** the overrides file with the submitted set
/// verbatim. **Admin only.**
///
/// Any allowlisted key omitted from the body is dropped from the file — this is
/// how the editor deletes an override (and bundles edits + deletes in one Save).
/// Does **not** restart: the new values take effect only after
/// `POST /server/restart`. `409` if overrides are disabled. The `Body` extractor
/// enforces the allowlist (only [`ConfigOverridesData`] fields deserialize) and
/// runs its validators (e.g. playback-complete 0-100).
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
mod tests {
    use crate::routers::playbacks::tests::{build_test_router, generate_jwt_token, setup_test_db};
    use crate::tests::harness::json_body;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    async fn get_config(router: &axum::Router, token: Option<&str>) -> axum::response::Response {
        let mut b = Request::builder().method("GET").uri("/api/v1/admin/config");
        if let Some(t) = token {
            b = b.header("Authorization", format!("Bearer {t}"));
        }
        router
            .clone()
            .oneshot(b.body(Body::empty()).unwrap())
            .await
            .unwrap()
    }

    /// No token → 401 (the route is behind the JWT layer).
    #[tokio::test]
    async fn config_requires_authentication() {
        let (_root, dbc, _payload) = setup_test_db().await;
        let router = build_test_router(dbc);
        assert_eq!(
            get_config(&router, None).await.status(),
            StatusCode::UNAUTHORIZED
        );
    }

    /// A non-admin authed user → 403.
    #[tokio::test]
    async fn config_forbidden_for_non_admin() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);
        let user = generate_jwt_token(payload["user_id"].as_str().unwrap());
        assert_eq!(
            get_config(&router, Some(&user)).await.status(),
            StatusCode::FORBIDDEN
        );
    }

    /// Admin gets the reconciled config, and the secret fields are absent from
    /// the JSON while a known non-secret field is present and correct.
    #[tokio::test]
    async fn config_admin_succeeds_and_omits_secrets() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);
        let admin = generate_jwt_token(payload["admin_id"].as_str().unwrap());

        let resp = get_config(&router, Some(&admin)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let json = json_body(resp).await;

        let data = &json["data"];
        assert!(data.is_object(), "config payload present");
        // A representative non-secret field comes through.
        assert!(
            data["listen_port"].is_number(),
            "listen_port should be reported"
        );
        // Secrets must never be serialised.
        assert!(
            data.get("auth_token_secret").is_none(),
            "auth_token_secret must be omitted; got: {data}"
        );
        assert!(
            data.get("admin_password").is_none(),
            "admin_password must be omitted; got: {data}"
        );
    }

    /// Issue a `/config-overrides` request with an optional token + body.
    async fn overrides_req(
        router: &axum::Router,
        method: &str,
        token: Option<&str>,
        body: Body,
    ) -> axum::response::Response {
        let mut b = Request::builder()
            .method(method)
            .uri("/api/v1/admin/config-overrides")
            .header("Content-Type", "application/json");
        if let Some(t) = token {
            b = b.header("Authorization", format!("Bearer {t}"));
        }
        router.clone().oneshot(b.body(body).unwrap()).await.unwrap()
    }

    /// No token → 401 (behind the JWT layer like GET /config).
    #[tokio::test]
    async fn overrides_get_requires_authentication() {
        let (_root, dbc, _payload) = setup_test_db().await;
        let router = build_test_router(dbc);
        assert_eq!(
            overrides_req(&router, "GET", None, Body::empty())
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );
    }

    /// A non-admin authed user is forbidden on every verb.
    #[tokio::test]
    async fn overrides_forbidden_for_non_admin() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);
        let user = generate_jwt_token(payload["user_id"].as_str().unwrap());
        assert_eq!(
            overrides_req(&router, "GET", Some(&user), Body::empty())
                .await
                .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            // AdminUser gate runs before the Body extractor, so the body is moot.
            overrides_req(&router, "POST", Some(&user), Body::from(r#"{"data":{}}"#))
                .await
                .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            overrides_req(&router, "DELETE", Some(&user), Body::empty())
                .await
                .status(),
            StatusCode::FORBIDDEN
        );
    }

    /// Admin GET (no overrides file resolved in the test config) → 200 with an
    /// empty object, the shape the editor prepopulates from.
    #[tokio::test]
    async fn overrides_admin_get_returns_object() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);
        let admin = generate_jwt_token(payload["admin_id"].as_str().unwrap());
        let resp = overrides_req(&router, "GET", Some(&admin), Body::empty()).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let json = json_body(resp).await;
        assert!(
            json["data"].is_object(),
            "overrides payload present; got: {json}"
        );
    }
}
