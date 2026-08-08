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

/// Shape a polling-control operation's result into the standard response: a
/// `PollingOperationData { message }` on success (logged at info), else an
/// `ApiError` keyed `request`/`<failure_code>` carrying `<failure_context>: <e>`.
///
/// `start`/`stop`/`poll` differ only in those strings and the failure code. The
/// code is a parameter on purpose, NOT an oversight: `start`/`stop` fail on a
/// state conflict (409 — already running / already stopped), whereas `poll` fails
/// at runtime mid-sync (500), so they legitimately map to different statuses.
pub(crate) fn op_result<E: std::fmt::Display>(
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
