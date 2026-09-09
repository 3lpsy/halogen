use crate::routers::{
    ApiError,
    extractors::{AuthUserId, Query},
};
use axum::{Extension, Json};
use halogen_wire::{ResponseData, SyncChangesData, SyncChangesParams};
use sea_orm::DatabaseConnection;

/// Read the authenticated actor's ordered resource changes and deletion markers.
pub async fn changes(
    Extension(db): Extension<DatabaseConnection>,
    AuthUserId(actor): AuthUserId,
    Query(params): Query<SyncChangesParams>,
) -> Result<Json<ResponseData<SyncChangesData>>, ApiError> {
    halogen_queries::sync::changes(&db, actor, &params)
        .await
        .map(ResponseData::from_data)
        .map(Json)
        .map_err(ApiError)
}
