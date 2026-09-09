use axum::{Json, extract::Extension};
use halogen_utils::constants::VALIDATION_PANIC_CODE;
use halogen_wire::{PollingOperationData, ResponseData};

use super::super::types::{AppState, op_result};
use crate::routers::errors::ApiError;
use crate::routers::extractors::AdminUser;

/// POST /poll - Force a one-shot poll of every feed (ignores intervals). **Admin only.**
#[axum::debug_handler]
pub async fn poll(
    Extension(state): Extension<AppState>,
    _admin: AdminUser,
) -> Result<Json<ResponseData<PollingOperationData>>, ApiError> {
    // PANIC (500), not CONFLICT: a poll fails at runtime mid-sync, not on a state
    // conflict like start/stop — see `op_result`.
    op_result(
        state.polling.poll().await,
        "Poll completed successfully",
        "Poll failed",
        VALIDATION_PANIC_CODE,
    )
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
