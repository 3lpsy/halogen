//! Admin-only server restart endpoint.
//!
//! `POST /api/v1/server/restart` asks the process to re-exec itself (see
//! [`RestartHandle`](crate::restart::RestartHandle)). The handler just
//! flags the request and returns success immediately; the actual graceful
//! shutdown + re-exec happens in `main` once in-flight requests drain.
//! This is deliberately decoupled from writing overrides
//! (`POST /config-overrides`) — an operator (or the UI button) writes config,
//! then triggers a restart separately to apply it.

use axum::{Extension, Json};
use halogen_wire::{PollingOperationData, ResponseData};
use tracing::info;

use crate::restart::RestartHandle;
use crate::routers::extractors::AdminUser;

/// POST /server/restart — request a graceful restart (re-exec). **Admin only.**
pub async fn request_restart(
    Extension(restart): Extension<RestartHandle>,
    _admin: AdminUser,
) -> Json<ResponseData<PollingOperationData>> {
    info!("Server restart requested via API");
    restart.request();
    Json(ResponseData::from_data(PollingOperationData {
        message: "Server restart requested".to_string(),
    }))
}
