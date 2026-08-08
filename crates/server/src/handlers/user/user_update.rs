use halogen_wire::{UserData, UserUpdateData, ValidationErrors};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter,
    Set,
};
use tracing::info;

use crate::handlers::db_error;
use halogen_orm::common::EntityHelpers;
use halogen_orm::user::{ActiveModel as UserActiveModel, Column, Entity as UserEntity};
use halogen_utils::constants::VALIDATION_INVALID_CODE;
use halogen_utils::verrors;

pub async fn handle(
    dbc: &DatabaseConnection,
    user_id: i32,
    update_data: &UserUpdateData,
) -> Result<UserData, ValidationErrors> {
    // The DTO was already validated by the `Body<UserUpdateData>` extractor.
    let existing = UserEntity::by_id_or_err(dbc, user_id).await?;

    // Demoting the last remaining admin would permanently lock everyone out of
    // the admin surface (there is no other way to mint an admin at runtime).
    if update_data.is_admin == Some(false) && existing.is_admin {
        let other_admins = UserEntity::find()
            .filter(Column::IsAdmin.eq(true))
            .filter(Column::Id.ne(user_id))
            .count(dbc)
            .await
            .map_err(db_error("counting remaining admins"))?;
        if other_admins == 0 {
            return Err(verrors(
                "is_admin",
                VALIDATION_INVALID_CODE,
                "Cannot remove admin status from the last admin".to_string(),
            ));
        }
    }

    let mut user: UserActiveModel = existing.into();
    if let Some(username) = &update_data.username {
        // Usernames are stored lowercase, whatever case the client submitted.
        user.username = Set(username.to_lowercase());
    }
    if let Some(is_admin) = update_data.is_admin {
        user.is_admin = Set(is_admin);
    }

    // `update` returns the updated row — no refetch needed.
    let updated = user.update(dbc).await.map_err(db_error("updating user"))?;

    info!("Updated user '{}'", updated.username);
    Ok(UserData::from(updated))
}
