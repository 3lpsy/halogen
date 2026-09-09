use axum::{Extension, Json};
use halogen_wire::{PasswordChangeData, ResponseData};
use sea_orm::DatabaseConnection;

use crate::handlers::user::password_change;
use crate::routers::ApiError;
use crate::routers::extractors::{AuthUserId, Body};

/// `POST /auth/password` — change the authenticated user's own password. The account is taken from the bearer
/// token (`AuthUserId`), never the body, so a user can only ever change their own password. The body carries
/// the current password (re-verified by the handler) plus the new password and its confirmation;
/// `Body<PasswordChangeData>` has already enforced the 8-char minimum and that the two new entries match.
pub async fn change_password(
    Extension(dbc): Extension<DatabaseConnection>,
    AuthUserId(user_id): AuthUserId,
    Body(data): Body<PasswordChangeData>,
) -> Result<Json<ResponseData<()>>, ApiError> {
    password_change::handle(&dbc, user_id, &data)
        .await
        .map_err(ApiError)?;
    Ok(Json(ResponseData::from_data(())))
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
