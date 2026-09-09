use axum::{Json, extract::Extension};
use halogen_wire::{ResponseData, UserData, UserShowParams};
use sea_orm::DatabaseConnection;

use crate::handlers::user::user_get;
use crate::routers::ApiError;
use crate::routers::extractors::{Actor, Id};
use crate::routers::guards;

/// Fetch a single user by ID. A non-admin may read only their own record.
pub async fn get(
    Extension(dbc): Extension<DatabaseConnection>,
    actor: Actor,
    Id(user_id): Id,
) -> Result<Json<ResponseData<UserData>>, ApiError> {
    guards::require_self_or_admin(actor, user_id)?;

    let params = UserShowParams {
        id: Some(user_id),
        username: None,
    };

    let user_data = user_get::handle(&dbc, &params).await.map_err(ApiError)?;
    Ok(Json(ResponseData::from_data(user_data)))
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
