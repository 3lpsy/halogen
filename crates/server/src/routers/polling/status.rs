use axum::{Json, extract::Extension};
use halogen_wire::{PollingStatusData, ResponseData};
use tracing::info;

use super::types::AppState;

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
mod tests {
    use crate::routers::playbacks::tests::{build_test_router, generate_jwt_token, setup_test_db};
    use crate::tests::harness::{authed, json_body, unauthed};
    use axum::http::StatusCode;
    use tower::ServiceExt;

    /// `/status` is authed — no token → 401.
    #[tokio::test]
    async fn status_requires_authentication() {
        let (_root, dbc, _payload) = setup_test_db().await;
        let router = build_test_router(dbc);
        let resp = router
            .oneshot(unauthed("GET", "/api/v1/status"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    /// Any authed (non-admin) user may read status; a fresh router isn't running.
    #[tokio::test]
    async fn status_ok_for_non_admin_and_reports_not_running() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);
        let user = generate_jwt_token(payload["user_id"].as_str().unwrap());
        let resp = router
            .oneshot(authed("GET", "/api/v1/status", &user))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "non-admin may read status");
        let json = json_body(resp).await;
        assert_eq!(json["data"]["running"].as_bool(), Some(false));
    }
}
