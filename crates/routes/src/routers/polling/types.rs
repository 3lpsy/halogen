use axum::Json;
use halogen_utils::constants::VALIDATION_REQUEST_FIELD;
use halogen_wire::{PollingOperationData, ResponseData};
use tracing::info;

use crate::routers::errors::ApiError;
use halogen_polling::PollingHandle;

/// Application state shared across all handlers.
#[derive(Debug, Clone)]
pub struct AppState {
    pub polling: PollingHandle,
}

/// Map polling-control success to PollingOperationData and info logging; failures become request-keyed ApiErrors. Keep
/// the failure code parameter: start/stop conflicts return 409, while runtime poll failures return 500.
pub fn op_result<E: std::fmt::Display>(
    result: Result<(), E>,
    success_message: &str,
    failure_context: &str,
    failure_code: &'static str,
) -> Result<Json<ResponseData<PollingOperationData>>, ApiError> {
    match result {
        Ok(()) => {
            info!("{success_message}");
            Ok(Json(ResponseData::from_data(PollingOperationData {
                message: success_message.to_string(),
            })))
        }
        Err(e) => Err(ApiError::new(
            VALIDATION_REQUEST_FIELD,
            failure_code,
            format!("{failure_context}: {e}"),
        )),
    }
}
