use serde::Deserialize;

use halogen_wire::SerializableValidationErrors;

/// Errors returned by the API client.
///
/// `Transport` is what the sync worker treats as "offline" to pause and retry.
/// `Validation` is the form-error path for login/setup screens.
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    /// Field-level validation errors from the server envelope or 4xx response.
    #[error("validation errors: {0:?}")]
    Validation(SerializableValidationErrors),

    /// Raw server error response (status + message), e.g. 401 invalid credentials.
    #[error("server error {}: {}", status, message)]
    Server { status: u16, message: String },

    /// Network error — transport layer failure (offline / unreachable).
    #[error("transport error: {0}")]
    Transport(#[from] reqwest::Error),

    /// Body could not be parsed into the expected shape.
    #[error("decode error: {0}")]
    Decode(String),

    /// 2xx response but `data` was `None` unexpectedly.
    #[error("response contained no data")]
    Empty,
}

impl ApiError {
    /// Classify unreachable-server failures as offline so callers trust cached data and show the offline state. Actual
    /// server responses need their own error message. Keep toast and list-state handling routed through this predicate.
    pub fn is_offline(&self) -> bool {
        matches!(self, ApiError::Transport(_))
    }
}

/// Parsed raw error body — either `{"error": "..."}` or the `ResponseData` envelope.
#[derive(Debug, Deserialize)]
struct RawErrorBody {
    error: Option<String>,
}

/// Partial deserialization of ResponseData envelope for error parsing.
#[derive(Debug, Deserialize)]
struct ErrorEnvelope {
    errors: Option<SerializableValidationErrors>,
}

/// Try to parse an error response body into an `ApiError`.
///
/// Tries `ResponseData.errors` first, then raw `{"error": "..."}`, else `Decode`.
pub fn parse_error_body(status: u16, bytes: &[u8]) -> Result<ApiError, ApiError> {
    // Try ResponseData envelope first.
    if let Ok(resp) = serde_json::from_slice::<ErrorEnvelope>(bytes)
        && let Some(errors) = resp.errors
    {
        return Err(ApiError::Validation(errors));
    }

    // Fallback: raw {"error": "..."} shape.
    if let Ok(raw) = serde_json::from_slice::<RawErrorBody>(bytes)
        && let Some(msg) = raw.error
    {
        return Err(ApiError::Server {
            status,
            message: msg,
        });
    }

    let body = String::from_utf8_lossy(bytes).to_string();
    Err(ApiError::Decode(format!(
        "status {}, body: {}",
        status, body
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The half that keeps a real failure visible: anything the server actually
    /// answered with must NOT be mistaken for "offline", or the list pages would
    /// silently swallow it and render a stale cache as if nothing were wrong.
    #[test]
    fn responses_the_server_produced_are_not_offline() {
        assert!(
            !ApiError::Server {
                status: 500,
                message: "boom".into(),
            }
            .is_offline(),
            "a 5xx came FROM the server — it is reachable"
        );
        assert!(
            !ApiError::Server {
                status: 404,
                message: "gone".into(),
            }
            .is_offline()
        );
        assert!(!ApiError::Decode("bad body".into()).is_offline());
        assert!(!ApiError::Empty.is_offline());
        assert!(
            !ApiError::Validation(SerializableValidationErrors {
                errors: std::collections::HashMap::new(),
            })
            .is_offline()
        );
    }

    /// The other half, against a genuine `reqwest::Error`: port 1 on loopback has
    /// nothing listening, so this is a real connection-refused transport failure
    /// (no network, no fixture) — the same shape as the server being unreachable
    /// because a VPN/tunnel dropped.
    #[cfg(not(target_arch = "wasm32"))]
    #[tokio::test]
    async fn unreachable_server_is_offline() {
        let err: ApiError = reqwest::get("http://127.0.0.1:1/")
            .await
            .expect_err("nothing listens on port 1")
            .into();
        assert!(
            err.is_offline(),
            "an unreachable server must classify as offline, got: {err}"
        );
    }

    #[test]
    fn parse_error_body_response_data_envelope() {
        let body = r#"{
            "data": null,
            "errors": {
                "username": [{"code": "length", "message": "too short"}]
            },
            "paginator": null
        }"#;
        let err = parse_error_body(400, body.as_bytes()).unwrap_err();
        match err {
            ApiError::Validation(errors) => {
                assert!(errors.errors.contains_key("username"));
            }
            other => panic!("expected Validation, got {:?}", other),
        }
    }

    #[test]
    fn parse_error_body_raw_error() {
        let body = r#"{"error": "Invalid credentials"}"#;
        let err = parse_error_body(401, body.as_bytes()).unwrap_err();
        match err {
            ApiError::Server {
                status: 401,
                message,
            } => {
                assert_eq!(message, "Invalid credentials");
            }
            other => panic!("expected Server, got {:?}", other),
        }
    }

    #[test]
    fn parse_error_body_decode_fallback() {
        let body = r#"not json at all"#;
        let err = parse_error_body(500, body.as_bytes()).unwrap_err();
        match err {
            ApiError::Decode(msg) => {
                assert!(msg.contains("500"));
            }
            other => panic!("expected Decode, got {:?}", other),
        }
    }
}
