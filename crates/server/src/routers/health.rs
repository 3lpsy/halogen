//! Unauthenticated liveness probe.
//!
//! `GET /healthz` is mounted at the app **root** (outside `/api/v1` and outside
//! all auth layers) so infra (load balancers, k8s probes) and the client's
//! pre-auth connect-form check can confirm the server is reachable without a
//! token. The polling `/status` endpoint is authed and is *not* a health probe.

use axum::Json;
use halogen_wire::{ResponseData, StatusData, VersionData};

/// GET /healthz — always 200 while the process can serve requests. No auth.
pub async fn healthz() -> Json<ResponseData<StatusData>> {
    Json(ResponseData::from_data(StatusData { running: true }))
}

/// GET /api/v1/version — the running server's build version (workspace
/// `Cargo.toml` version, baked in at compile time). Public, no auth: clients
/// use it pre-login to detect stale deployments.
pub async fn version() -> Json<ResponseData<VersionData>> {
    Json(ResponseData::from_data(VersionData {
        version: env!("CARGO_PKG_VERSION").to_string(),
    }))
}

#[cfg(test)]
mod tests {
    use crate::routers::playbacks::tests::{build_test_router, setup_test_db};
    use crate::tests::harness::{json_body, unauthed};
    use axum::http::StatusCode;
    use tower::ServiceExt;

    /// `/healthz` is reachable with no token and 200s — the public probe.
    #[tokio::test]
    async fn healthz_is_public_and_ok() {
        let (_root, dbc, _payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let response = router.oneshot(unauthed("GET", "/healthz")).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let json = json_body(response).await;
        assert_eq!(json["data"]["running"].as_bool(), Some(true));
    }

    /// `/api/v1/version` is reachable with no token and reports the crate version.
    #[tokio::test]
    async fn version_is_public_and_reports_cargo_version() {
        let (_root, dbc, _payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let response = router
            .oneshot(unauthed("GET", "/api/v1/version"))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let json = json_body(response).await;
        assert_eq!(
            json["data"]["version"].as_str(),
            Some(env!("CARGO_PKG_VERSION"))
        );
    }
}
