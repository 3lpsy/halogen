use axum::{Json, extract::Extension};
use halogen_utils::constants::VALIDATION_CONFLICT_CODE;
use halogen_wire::{PollingOperationData, ResponseData};

use super::types::{AppState, op_result};
use crate::routers::errors::ApiError;
use crate::routers::extractors::AdminUser;

/// POST /stop - Stops the polling service. **Admin only.**
#[axum::debug_handler]
pub async fn stop(
    Extension(state): Extension<AppState>,
    _admin: AdminUser,
) -> Result<Json<ResponseData<PollingOperationData>>, ApiError> {
    op_result(
        state.polling.stop(),
        "Polling service stopped",
        "Failed to stop polling service",
        VALIDATION_CONFLICT_CODE,
    )
}

#[cfg(test)]
mod tests {
    use crate::routers::playbacks::tests::{build_test_router, generate_jwt_token, setup_test_db};
    use crate::tests::harness::json_body;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    async fn req(
        router: &axum::Router,
        method: &str,
        uri: &str,
        token: Option<&str>,
    ) -> axum::response::Response {
        let mut b = Request::builder().method(method).uri(uri);
        if let Some(t) = token {
            b = b.header("Authorization", format!("Bearer {t}"));
        }
        router
            .clone()
            .oneshot(b.body(Body::empty()).unwrap())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn stop_forbidden_for_non_admin() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);
        let user = generate_jwt_token(payload["user_id"].as_str().unwrap());
        assert_eq!(
            req(&router, "POST", "/api/v1/admin/stop", Some(&user))
                .await
                .status(),
            StatusCode::FORBIDDEN
        );
    }

    /// Admin stop succeeds, and `/status` then reports not-running again.
    #[tokio::test]
    async fn stop_admin_succeeds_and_status_reflects_not_running() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);
        let admin = generate_jwt_token(payload["admin_id"].as_str().unwrap());

        // Start first so there's a task to stop.
        assert_eq!(
            req(&router, "POST", "/api/v1/admin/start", Some(&admin))
                .await
                .status(),
            StatusCode::OK
        );

        let stop = req(&router, "POST", "/api/v1/admin/stop", Some(&admin)).await;
        assert_eq!(stop.status(), StatusCode::OK);
        let json = json_body(stop).await;
        assert_eq!(
            json["data"]["message"].as_str(),
            Some("Polling service stopped")
        );

        let status = req(&router, "GET", "/api/v1/status", Some(&admin)).await;
        let json = json_body(status).await;
        assert_eq!(json["data"]["running"].as_bool(), Some(false));
    }
}
