use axum::{Json, extract::Extension};
use halogen_utils::constants::VALIDATION_CONFLICT_CODE;
use halogen_wire::{PollingOperationData, ResponseData};

use super::super::types::{AppState, op_result};
use crate::routers::errors::ApiError;
use crate::routers::extractors::AdminUser;

/// POST /start - Starts the polling service. **Admin only.**
#[axum::debug_handler]
pub async fn start(
    Extension(state): Extension<AppState>,
    _admin: AdminUser,
) -> Result<Json<ResponseData<PollingOperationData>>, ApiError> {
    op_result(
        state.polling.start(),
        "Polling service started",
        "Failed to start polling service",
        VALIDATION_CONFLICT_CODE,
    )
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
