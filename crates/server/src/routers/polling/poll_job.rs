use axum::{Json, extract::Extension};
use halogen_utils::constants::{
    VALIDATION_EXISTS_CODE, VALIDATION_ID_FIELD, VALIDATION_PANIC_CODE,
};
use halogen_wire::{PollJobData, PollJobStartData, ResponseData};
use serde::Deserialize;
use serde_qs::axum::QsQuery;
use tracing::info;

use super::types::AppState;
use crate::routers::errors::ApiError;
use crate::routers::extractors::{AdminUser, Id};

/// Optional query for `POST /admin/poll-job`: `?podcast_id=7` scopes the run to
/// one feed (the podcast-detail page); absent = poll all feeds.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct PollJobQuery {
    pub podcast_id: Option<i32>,
}

/// POST /admin/poll-job — start an on-demand poll job and return its id once the
/// job row is persisted. The feed sync runs in the background; clients poll
/// `GET /admin/poll-job/{id}` for progress. **Admin only.**
#[axum::debug_handler]
pub async fn start_poll_job(
    Extension(state): Extension<AppState>,
    _admin: AdminUser,
    QsQuery(q): QsQuery<PollJobQuery>,
) -> Result<Json<ResponseData<PollJobStartData>>, ApiError> {
    let job_id = state
        .polling
        .spawn_poll_job(q.podcast_id)
        .await
        .map_err(|e| ApiError::new("poll_job", VALIDATION_PANIC_CODE, e))?;
    info!(job_id, podcast_id = ?q.podcast_id, "Poll job started via API");
    Ok(Json(ResponseData::from_data(PollJobStartData { job_id })))
}

/// GET /admin/poll-job/{id} — snapshot of one poll job. 404 once it has been
/// pruned from the DB-backed history. **Admin only.**
#[axum::debug_handler]
pub async fn get_poll_job(
    Extension(state): Extension<AppState>,
    _admin: AdminUser,
    Id(job_id): Id,
) -> Result<Json<ResponseData<PollJobData>>, ApiError> {
    match state.polling.jobs().get(job_id as u64).await {
        Some(job) => Ok(Json(ResponseData::from_data(job))),
        None => Err(ApiError::new(
            VALIDATION_ID_FIELD,
            VALIDATION_EXISTS_CODE,
            format!("Poll job {job_id} not found"),
        )),
    }
}

/// GET /admin/poll-jobs — recent poll jobs, newest first (capped). **Admin only.**
#[axum::debug_handler]
pub async fn list_poll_jobs(
    Extension(state): Extension<AppState>,
    _admin: AdminUser,
) -> Json<ResponseData<Vec<PollJobData>>> {
    Json(ResponseData::from_data(state.polling.jobs().recent().await))
}

#[cfg(test)]
mod tests {
    use crate::routers::playbacks::tests::{build_test_router, generate_jwt_token, setup_test_db};
    use crate::tests::harness::json_body;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    async fn send(
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
    async fn start_requires_authentication() {
        let (_root, dbc, _payload) = setup_test_db().await;
        let router = build_test_router(dbc);
        assert_eq!(
            send(&router, "POST", "/api/v1/admin/poll-job", None)
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );
    }

    #[tokio::test]
    async fn start_forbidden_for_non_admin() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);
        let user = generate_jwt_token(payload["user_id"].as_str().unwrap());
        assert_eq!(
            send(&router, "POST", "/api/v1/admin/poll-job", Some(&user))
                .await
                .status(),
            StatusCode::FORBIDDEN
        );
    }

    #[tokio::test]
    async fn admin_starts_job_and_lists_it() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);
        let admin = generate_jwt_token(payload["admin_id"].as_str().unwrap());

        let resp = send(&router, "POST", "/api/v1/admin/poll-job", Some(&admin)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let json = json_body(resp).await;
        let job_id = json["data"]["job_id"].as_i64().expect("job_id present");
        assert!(job_id >= 1);

        // The job shows up in the history list.
        let resp = send(&router, "GET", "/api/v1/admin/poll-jobs", Some(&admin)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let json = json_body(resp).await;
        let jobs = json["data"].as_array().expect("array");
        assert!(jobs.iter().any(|j| j["id"].as_i64() == Some(job_id)));
    }

    #[tokio::test]
    async fn get_unknown_job_is_404() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);
        let admin = generate_jwt_token(payload["admin_id"].as_str().unwrap());
        assert_eq!(
            send(&router, "GET", "/api/v1/admin/poll-job/99999", Some(&admin))
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
    }
}
