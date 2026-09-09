use axum::{Extension, Json};
use halogen_utils::constants::*;
use halogen_wire::{ResponseData, UserDeleteParams};
use sea_orm::DatabaseConnection;
use tracing::warn;

use crate::handlers::user::user_delete;
use crate::routers::errors::ApiError;
use crate::routers::extractors::{AdminUser, Id};

/// Delete a user. Admin-only; an admin may not delete their own account.
pub async fn delete(
    Extension(dbc): Extension<DatabaseConnection>,
    AdminUser(admin_id): AdminUser,
    Id(user_id): Id,
) -> Result<Json<ResponseData<()>>, ApiError> {
    if admin_id == user_id {
        warn!("Admin '{}' attempted to delete their own account", admin_id);
        return Err(ApiError::new(
            VALIDATION_ID_FIELD,
            VALIDATION_INVALID_CODE,
            "Cannot delete yourself".to_string(),
        ));
    }

    let params = UserDeleteParams { id: user_id };
    user_delete::handle(&dbc, &params).await.map_err(|err| {
        warn!("Error deleting user: {:?}", err);
        ApiError(err)
    })?;
    Ok(Json(ResponseData::from_data(())))
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
