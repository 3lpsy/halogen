//! Unauthenticated liveness probe. `GET /healthz` is mounted at the app **root** (outside `/api/v1` and
//! outside all auth layers) so infra (load balancers, k8s probes) and the client's pre-auth connect-form check
//! can confirm the server is reachable without a token. The polling `/status` endpoint is authed and is *not* a
//! health probe.

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
#[path = "tests.rs"]
mod tests;
