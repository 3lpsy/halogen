use axum::{Json, extract::Extension};
use halogen_utils::constants::{VALIDATION_DATA_FIELD, VALIDATION_INVALID_CODE};
use halogen_wire::{OpmlImportData, OpmlImportResultData, ResponseData};
use sea_orm::DatabaseConnection;
use tracing::{info, warn};

use crate::routers::errors::ApiError;
use crate::routers::extractors::{AdminUser, Body};
use crate::routers::polling::types::AppState;
use halogen_opml::import_podcasts_from_opml_str;

/// POST /opml/import — import podcasts from uploaded OPML XML. **Admin only.** Inserts the podcasts (bare
/// title + feed_url, deduped by feed URL) and returns a count summary immediately. A feed sync is fired in the
/// background so the newly-added podcasts get their episodes; the response does not wait for it.
pub async fn import(
    Extension(dbc): Extension<DatabaseConnection>,
    Extension(state): Extension<AppState>,
    AdminUser(owner_id): AdminUser,
    Body(data): Body<OpmlImportData>,
) -> Result<Json<ResponseData<OpmlImportResultData>>, ApiError> {
    // Imported podcasts are owned by the importing admin.
    let result = import_podcasts_from_opml_str(&dbc, &data.opml, owner_id)
        .await
        .map_err(|e| ApiError::new(VALIDATION_DATA_FIELD, VALIDATION_INVALID_CODE, e))?;

    info!(
        "OPML import via API: {} created, {} skipped, {} errors",
        result.created, result.skipped, result.errors
    );

    // Populate episodes for the freshly-inserted podcasts without blocking the
    // response. `poll()` syncs all podcasts (idempotent); spawn it detached.
    if result.created > 0 {
        let polling = state.polling.clone();
        tokio::spawn(async move {
            if let Err(e) = polling.poll().await {
                warn!("Post-import feed sync failed: {}", e);
            }
        });
    }

    Ok(Json(ResponseData::from_data(OpmlImportResultData {
        created: result.created,
        skipped: result.skipped,
        errors: result.errors,
    })))
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
