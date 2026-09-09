use axum::{Extension, Json};
use halogen_wire::{ResponseData, UserData, UserStoreData};
use sea_orm::DatabaseConnection;

use crate::handlers::user::user_store;
use crate::routers::ApiError;
use crate::routers::extractors::{AdminUser, Body};

/// Create a user through the admin-only `/admin/users` route for account provisioning.
pub async fn store(
    Extension(dbc): Extension<DatabaseConnection>,
    _admin: AdminUser,
    Body(data): Body<UserStoreData>,
) -> Result<Json<ResponseData<UserData>>, ApiError> {
    let user_data = user_store::handle(&dbc, &data).await.map_err(ApiError)?;
    Ok(Json(ResponseData::from_data(user_data)))
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
