use axum::{Extension, Json};
use halogen_utils::constants::{VALIDATION_REQUEST_FIELD, VALIDATION_UNAUTHORIZED_CODE};
use halogen_wire::{ResponseData, UserData, UserUpdateData};
use sea_orm::DatabaseConnection;

use crate::handlers::user::user_update;
use crate::routers::ApiError;
use crate::routers::extractors::{Actor, Body, Id};
use crate::routers::guards;

pub async fn update(
    Extension(dbc): Extension<DatabaseConnection>,
    actor: Actor,
    Id(user_id): Id,
    Body(update_data): Body<UserUpdateData>,
) -> Result<Json<ResponseData<UserData>>, ApiError> {
    // A non-admin may update only their own record.
    guards::require_self_or_admin(actor, user_id)?;

    // Only an admin may grant/revoke admin status — otherwise a self-update is a
    // privilege-escalation vector.
    if update_data.is_admin.is_some() && !actor.is_admin {
        return Err(ApiError::new(
            VALIDATION_REQUEST_FIELD,
            VALIDATION_UNAUTHORIZED_CODE,
            "Only an admin may change admin status".to_string(),
        ));
    }

    let user_data = user_update::handle(&dbc, user_id, &update_data)
        .await
        .map_err(ApiError)?;
    Ok(Json(ResponseData::from_data(user_data)))
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
