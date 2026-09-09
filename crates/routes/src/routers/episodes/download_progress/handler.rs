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
#[path = "tests.rs"]
mod tests;
