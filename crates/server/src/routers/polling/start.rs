use axum::{Json, extract::Extension};
use halogen_utils::constants::VALIDATION_CONFLICT_CODE;
use halogen_wire::{PollingOperationData, ResponseData};

use super::types::{AppState, op_result};
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
mod tests {
    use crate::routers::playbacks::tests::{build_test_router, generate_jwt_token, setup_test_db};
    use crate::tests::harness::{authed, json_body};
    use axum::http::StatusCode;
    use tower::ServiceExt;

    async fn post(router: &axum::Router, uri: &str, token: &str) -> axum::response::Response {
        router
            .clone()
            .oneshot(authed("POST", uri, token))
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn start_forbidden_for_non_admin() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);
        let user = generate_jwt_token(payload["user_id"].as_str().unwrap());
        assert_eq!(
            post(&router, "/api/v1/admin/start", &user).await.status(),
            StatusCode::FORBIDDEN
        );
    }

    /// Admin start succeeds, and `/status` then reports running (the zero-interval
    /// test handle stays running until stopped).
    #[tokio::test]
    async fn start_admin_succeeds_and_status_reflects_running() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);
        let admin = generate_jwt_token(payload["admin_id"].as_str().unwrap());

        let start = post(&router, "/api/v1/admin/start", &admin).await;
        assert_eq!(start.status(), StatusCode::OK);
        let json = json_body(start).await;
        assert_eq!(
            json["data"]["message"].as_str(),
            Some("Polling service started")
        );

        let status = router
            .oneshot(authed("GET", "/api/v1/status", &admin))
            .await
            .unwrap();
        let json = json_body(status).await;
        assert_eq!(json["data"]["running"].as_bool(), Some(true));
    }
}
