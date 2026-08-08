use axum::{Json, extract::Extension};
use halogen_utils::constants::VALIDATION_PANIC_CODE;
use halogen_wire::{PollingOperationData, ResponseData};

use super::types::{AppState, op_result};
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
mod tests {
    use crate::routers::playbacks::tests::{build_test_router, generate_jwt_token, setup_test_db};
    use crate::tests::harness::json_body;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    async fn poll(router: &axum::Router, token: Option<&str>) -> axum::response::Response {
        let mut b = Request::builder().method("POST").uri("/api/v1/admin/poll");
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
    async fn poll_requires_authentication() {
        let (_root, dbc, _payload) = setup_test_db().await;
        let router = build_test_router(dbc);
        assert_eq!(poll(&router, None).await.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn poll_forbidden_for_non_admin() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);
        let user = generate_jwt_token(payload["user_id"].as_str().unwrap());
        assert_eq!(
            poll(&router, Some(&user)).await.status(),
            StatusCode::FORBIDDEN
        );
    }

    #[tokio::test]
    async fn poll_admin_succeeds() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);
        let admin = generate_jwt_token(payload["admin_id"].as_str().unwrap());
        let resp = poll(&router, Some(&admin)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let json = json_body(resp).await;
        assert_eq!(
            json["data"]["message"].as_str(),
            Some("Poll completed successfully")
        );
    }
}
