//! Admin restart marks a request and returns immediately; main drains in-flight requests before re-exec.
//! Config-overrides writes remain separate so operators explicitly choose when to apply changes.

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
