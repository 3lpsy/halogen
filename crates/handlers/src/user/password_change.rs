use halogen_wire::{PasswordChangeData, ValidationErrors};
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set};
use tracing::{info, warn};

use crate::{db_error, not_found};
use halogen_orm::user::{ActiveModel as UserActiveModel, Column, Entity as UserEntity};
use halogen_utils::constants::*;
use halogen_utils::verrors;

/// Change the authenticated user's password. Re-verifies `current_password` against the stored bcrypt hash,
/// then stores a fresh hash of the new password. The new password's length and the confirmation match are
/// enforced up-front by the `Body<PasswordChangeData>` extractor in the router, so the DTO arrives here already
/// valid.
pub async fn handle(
    dbc: &DatabaseConnection,
    user_id: i32,
    data: &PasswordChangeData,
) -> Result<(), ValidationErrors> {
    let user = UserEntity::find()
        .filter(Column::Id.eq(user_id))
        .one(dbc)
        .await
        .map_err(db_error("fetching user for password change"))?
        .ok_or_else(|| not_found("User not found"))?;

    // Being logged in isn't enough to rotate the credential — re-verify the
    // current password first.
    if !bcrypt::verify(&data.current_password, &user.password_hash).unwrap_or(false) {
        return Err(verrors(
            "current_password",
            VALIDATION_UNAUTHENTICATED_CODE,
            "Current password is incorrect".to_string(),
        ));
    }

    let new_hash =
        bcrypt::hash(&data.new_password.password, bcrypt::DEFAULT_COST).map_err(|e| {
            warn!("Failed to hash new password: {}", e);
            verrors(
                VALIDATION_REQUEST_FIELD,
                VALIDATION_PANIC_CODE,
                "Failed to hash password".to_string(),
            )
        })?;

    let mut active: UserActiveModel = user.into();
    active.password_hash = Set(new_hash);
    active.updated_at = Set(chrono::Utc::now());
    active
        .update(dbc)
        .await
        .map_err(db_error("updating password"))?;

    info!("Password changed for user {}", user_id);
    Ok(())
}
