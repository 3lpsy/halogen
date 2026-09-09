use axum::{Json, extract::Extension};
use halogen_wire::{PollingStatusData, ResponseData};
use tracing::info;

use super::super::types::AppState;

/// GET /status - Polling service status. Any authed user (read).
#[axum::debug_handler]
pub async fn status(
    Extension(state): Extension<AppState>,
) -> Json<ResponseData<PollingStatusData>> {
    let running = state.polling.is_running();
    info!("Polling status requested: running={}", running);
    Json(ResponseData::from_data(PollingStatusData { running }))
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
