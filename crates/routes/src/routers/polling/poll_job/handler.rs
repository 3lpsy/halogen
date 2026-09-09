use axum::{Json, extract::Extension};
use halogen_utils::constants::{
    VALIDATION_EXISTS_CODE, VALIDATION_ID_FIELD, VALIDATION_PANIC_CODE,
};
use halogen_wire::{PollJobData, PollJobStartData, ResponseData};
use serde::Deserialize;
use serde_qs::axum::QsQuery;
use tracing::info;

use super::super::types::AppState;
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
#[path = "tests.rs"]
mod tests;
