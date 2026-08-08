use axum::{Extension, Json};
use halogen_utils::constants::{VALIDATION_EXISTS_CODE, VALIDATION_ID_FIELD};
use halogen_wire::{DownloadProgressData, ResponseData};
use sea_orm::DatabaseConnection;

use crate::routers::ApiError;
use crate::routers::extractors::{Actor, Id};
use crate::routers::guards;
use crate::routers::polling::AppState;

/// GET /episodes/{id}/download-progress — live byte progress of an in-flight
/// download. **404** when no download is currently running for the episode (the
/// durable outcome lives on `episode.download_status`). Same subscription gate as
/// `POST /episodes/{id}/download`.
pub async fn download_progress(
    Extension(dbc): Extension<DatabaseConnection>,
    Extension(state): Extension<AppState>,
    actor: Actor,
    id: Id,
) -> Result<Json<ResponseData<DownloadProgressData>>, ApiError> {
    guards::require_episode_subscribed(&dbc, actor, id.0).await?;
    match state.polling.download_tracker().get(id.0) {
        Some(progress) => Ok(Json(ResponseData::from_data(progress))),
        None => Err(ApiError::new(
            VALIDATION_ID_FIELD,
            VALIDATION_EXISTS_CODE,
            "No download in progress for this episode".to_string(),
        )),
    }
}

/// GET /episodes/download-progress — every in-flight download. Any authenticated
/// caller may read it (progress is not sensitive); drives a global activity view.
pub async fn download_progress_active(
    Extension(state): Extension<AppState>,
    _actor: Actor,
) -> Json<ResponseData<Vec<DownloadProgressData>>> {
    Json(ResponseData::from_data(
        state.polling.download_tracker().active(),
    ))
}

#[cfg(test)]
mod tests {
    use crate::routers::episodes::tests::{build_test_router, generate_jwt_token, setup_test_db};
    use crate::tests::harness::{authed, unauthed};
    use axum::http::StatusCode;
    use tower::ServiceExt;

    // No download is ever in flight in the test router (a fresh, empty tracker), so
    // the per-episode endpoint 404s even for a subscribed episode.
    #[tokio::test]
    async fn per_episode_404_when_not_in_flight() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);
        let token = generate_jwt_token(payload.get("user_id").unwrap().as_str().unwrap());
        let episode_id = payload.get("episode_id_1").unwrap().as_str().unwrap();

        let resp = router
            .clone()
            .oneshot(authed(
                "GET",
                format!("/api/v1/episodes/{episode_id}/download-progress"),
                &token,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    // The active-list endpoint returns 200 (an empty list) for any authed caller.
    #[tokio::test]
    async fn active_list_ok_for_authed() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);
        let token = generate_jwt_token(payload.get("user_id").unwrap().as_str().unwrap());

        let resp = router
            .clone()
            .oneshot(authed("GET", "/api/v1/episodes/download-progress", &token))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    // Both endpoints sit behind the JWT layer.
    #[tokio::test]
    async fn active_list_requires_auth() {
        let (_root, dbc, _payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let resp = router
            .clone()
            .oneshot(unauthed("GET", "/api/v1/episodes/download-progress"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }
}
