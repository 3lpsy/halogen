//! Unified API error handling.
//!
//! Provides a single `ApiError` type that implements `IntoResponse`,
//! replacing the ad-hoc `{"error":"..."}` JSON responses scattered
//! across auth, polling, and middleware routes.

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use halogen_utils::constants::{
    VALIDATION_AUTHTOKEN_FIELD, VALIDATION_ID_FIELD, VALIDATION_INVALID_CODE,
    VALIDATION_PARSING_CODE, VALIDATION_REQUEST_FIELD, VALIDATION_UNAUTHENTICATED_CODE,
    VALIDATION_UNAUTHORIZED_CODE,
};
use halogen_utils::extract_status_code;
use halogen_wire::{ResponsableData, ResponseData, ValidationErrors};

/// Unified API error type.
///
/// Every handler, router, and middleware should return this instead of
/// raw `(StatusCode, Json<serde_json::Value>)` tuples.
#[derive(Debug, Clone)]
pub struct ApiError(pub ValidationErrors);

impl From<ValidationErrors> for ApiError {
    fn from(errs: ValidationErrors) -> Self {
        Self(errs)
    }
}

impl ApiError {
    /// Create a new `ApiError` with a single field error.
    pub fn new(field: &'static str, code: &'static str, message: String) -> Self {
        use halogen_utils::verrors;
        Self(verrors(field, code, message))
    }

    /// A path id that parsed but is out of range (`<= 0`, above `i32::MAX`, …).
    pub fn invalid_id(msg: String) -> Self {
        Self::new(VALIDATION_ID_FIELD, VALIDATION_INVALID_CODE, msg)
    }

    /// A path id that couldn't be parsed as an integer at all.
    pub fn unparsable_id(msg: String) -> Self {
        Self::new(VALIDATION_ID_FIELD, VALIDATION_PARSING_CODE, msg)
    }

    /// No / invalid credentials on the request's bearer token (→ 401).
    pub fn unauthenticated(msg: String) -> Self {
        Self::new(
            VALIDATION_AUTHTOKEN_FIELD,
            VALIDATION_UNAUTHENTICATED_CODE,
            msg,
        )
    }

    /// Authenticated but not permitted (→ 403).
    pub fn unauthorized(msg: String) -> Self {
        Self::new(VALIDATION_REQUEST_FIELD, VALIDATION_UNAUTHORIZED_CODE, msg)
    }

    /// Return the status code this error should map to.
    pub fn status(&self) -> StatusCode {
        let code = extract_status_code(&self.0);
        StatusCode::from_u16(code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
    }

    /// Convert this error into an Axum `Response`.
    ///
    /// The response body is a `ResponseData<()>` containing the validation
    /// errors — the same shape used by the standard handler envelope.
    pub fn into_response<T: ResponsableData>(self) -> Response {
        let status = self.status();
        let body = ResponseData::<T>::from(self.0);
        (status, axum::Json(body)).into_response()
    }
}

impl<T: ResponsableData> From<ApiError> for (StatusCode, axum::Json<ResponseData<T>>) {
    fn from(err: ApiError) -> Self {
        (err.status(), axum::Json(ResponseData::<T>::from(err.0)))
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = self.status();
        let body = ResponseData::<()>::from(self.0);
        (status, axum::Json(body)).into_response()
    }
}
