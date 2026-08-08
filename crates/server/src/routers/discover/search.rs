use std::sync::Arc;

use axum::{Extension, Json};
use halogen_utils::constants::VALIDATION_INVALID_CODE;
use halogen_wire::{DiscoverSearchData, DiscoverSearchParams, ResponseData};
use serde_qs::axum::QsQuery;

use crate::routers::errors::ApiError;
use halogen_discover::DiscoverService;

/// `GET /discover/search` — fan a query out to the selected providers.
///
/// Only an empty/whitespace `q` is a request error (400). Provider failures are
/// not: they ride back inside `data.errors` with a 200, so a flaky provider
/// never blanks the whole search.
pub async fn search(
    Extension(discover): Extension<Arc<DiscoverService>>,
    QsQuery(params): QsQuery<DiscoverSearchParams>,
) -> Result<Json<ResponseData<DiscoverSearchData>>, ApiError> {
    let q = params.q.trim();
    if q.is_empty() {
        return Err(ApiError::new(
            "q",
            VALIDATION_INVALID_CODE,
            "Search query is required".to_string(),
        ));
    }

    let data = discover.search(q, params.providers.as_deref()).await;
    Ok(Json(ResponseData::from_data(data)))
}
