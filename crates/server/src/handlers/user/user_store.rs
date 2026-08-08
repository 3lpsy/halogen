use chrono::Utc;
use halogen_wire::{UserData, UserStoreData, ValidationErrors};
use sea_orm::{ActiveModelTrait, DatabaseConnection, Set};
use tracing::{info, warn};

use crate::handlers::db_error;
use halogen_orm::user::ActiveModel as UserActiveModel;
use halogen_utils::constants::{VALIDATION_PANIC_CODE, VALIDATION_REQUEST_FIELD};
use halogen_utils::verrors;

/// Create a user (admin-only at the router). The DTO arrives validated by the
/// `Body<UserStoreData>` extractor (length bounds + password/confirm match);
/// duplicate usernames surface through the shared DB-error mapping as a
/// `unique` validation error on the column.
pub async fn handle(
    dbc: &DatabaseConnection,
    data: &UserStoreData,
) -> Result<UserData, ValidationErrors> {
    let password_hash = bcrypt::hash(&data.password, bcrypt::DEFAULT_COST).map_err(|e| {
        warn!("Failed to hash new user's password: {}", e);
        verrors(
            VALIDATION_REQUEST_FIELD,
            VALIDATION_PANIC_CODE,
            "Failed to hash password".to_string(),
        )
    })?;

    // Explicit id: the seeded admin's i32::MAX sentinel makes sqlite's
    // implicit rowid successor overflow (see `next_available_id`).
    let id = halogen_orm::user::next_available_id(dbc)
        .await
        .map_err(db_error("allocating a user id"))?;
    let now = Utc::now();
    let user = UserActiveModel {
        id: Set(id),
        // Usernames are stored lowercase, whatever case the client submitted
        // (login lowercases before lookup).
        username: Set(data.username.to_lowercase()),
        password_hash: Set(password_hash),
        is_admin: Set(data.is_admin.unwrap_or(false)),
        created_at: Set(now),
        updated_at: Set(now),
    };

    let created = user.insert(dbc).await.map_err(db_error("creating user"))?;

    info!("Created user '{}'", created.username);
    Ok(UserData::from(created))
}
