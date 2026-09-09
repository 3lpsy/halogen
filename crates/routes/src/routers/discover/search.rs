use std::sync::Arc;

use axum::{Extension, Json};
use halogen_utils::constants::VALIDATION_INVALID_CODE;
use halogen_wire::{DiscoverSearchData, DiscoverSearchParams, ResponseData};
use serde_qs::axum::QsQuery;

use crate::routers::errors::ApiError;
use halogen_discover::DiscoverService;

/// `GET /discover/search` — fan a query out to the selected providers. Only an empty/whitespace `q` is a
/// request error (400). Provider failures are not: they ride back inside `data.errors` with a 200, so a flaky
/// provider never blanks the whole search.
pub async fn search(
    Extension(discover): Extension<Arc<DiscoverService>>,
    QsQuery(params): QsQuery<DiscoverSearchParams>,
) -> Result<Json<ResponseData<DiscoverSearchData>>, ApiError> {
    let q = params.q.trim();
    if q.is_empty() || q.chars().count() > 256 {
        return Err(ApiError::new(
            "q",
            VALIDATION_INVALID_CODE,
            "Search query must contain 1–256 characters".to_string(),
        ));
    }

    let data = discover.search(q, params.providers.as_deref()).await;
    Ok(Json(ResponseData::from_data(data)))
}

/// Search remote episodes using the enabled provider subset.
pub async fn episodes(
    Extension(discover): Extension<Arc<DiscoverService>>,
    QsQuery(params): QsQuery<DiscoverSearchParams>,
) -> Result<Json<ResponseData<halogen_wire::DiscoverEpisodeSearchData>>, ApiError> {
    let q = params.q.trim();
    if q.is_empty() || q.chars().count() > 256 {
        return Err(ApiError::new(
            "q",
            VALIDATION_INVALID_CODE,
            "Search query must contain 1–256 characters".into(),
        ));
    }
    Ok(Json(ResponseData::from_data(
        discover
            .search_episodes(q, params.providers.as_deref())
            .await,
    )))
}

/// Read remote feed metadata without subscribing.
pub async fn podcast(
    Extension(discover): Extension<Arc<DiscoverService>>,
    QsQuery(params): QsQuery<halogen_wire::DiscoverPodcastParams>,
) -> Result<Json<ResponseData<halogen_wire::DiscoverPodcastData>>, ApiError> {
    let data = discover
        .podcast_preview(&params.feed_url, params.provider)
        .await
        .map_err(|message| ApiError::new("feed_url", VALIDATION_INVALID_CODE, message))?;
    Ok(Json(ResponseData::from_data(data)))
}

pub async fn podcast_page(
    Extension(discover): Extension<Arc<DiscoverService>>,
    QsQuery(params): QsQuery<halogen_wire::DiscoverPageParams>,
) -> Result<Json<ResponseData<halogen_wire::DiscoverPodcastPageData>>, ApiError> {
    let data = discover
        .podcast_page(params)
        .await
        .map_err(|message| ApiError::new("search", VALIDATION_INVALID_CODE, message))?;
    Ok(Json(ResponseData::from_data(data)))
}

pub async fn episode_page(
    Extension(discover): Extension<Arc<DiscoverService>>,
    QsQuery(params): QsQuery<halogen_wire::DiscoverPageParams>,
) -> Result<Json<ResponseData<halogen_wire::DiscoverEpisodePageData>>, ApiError> {
    let data = discover
        .episode_page(params)
        .await
        .map_err(|message| ApiError::new("search", VALIDATION_INVALID_CODE, message))?;
    Ok(Json(ResponseData::from_data(data)))
}
