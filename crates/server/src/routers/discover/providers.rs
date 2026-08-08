use std::sync::Arc;

use axum::{Extension, Json};
use halogen_wire::{DiscoverProvidersData, ResponseData};

use crate::routers::errors::ApiError;
use halogen_discover::DiscoverService;

/// `GET /discover/providers` — the provider list the UI renders toggle chips for.
pub async fn providers(
    Extension(discover): Extension<Arc<DiscoverService>>,
) -> Result<Json<ResponseData<DiscoverProvidersData>>, ApiError> {
    Ok(Json(ResponseData::from_data(discover.providers_info())))
}
