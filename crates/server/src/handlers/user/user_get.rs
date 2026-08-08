use halogen_wire::{UserData, UserShowParams, ValidationErrors};
use sea_orm::DatabaseConnection;
use tracing::info;
use validator::Validate;

use halogen_orm::common::EntityHelpers;
use halogen_orm::user::Entity as UserEntity;
use halogen_utils::constants::{VALIDATION_ID_FIELD, VALIDATION_INVALID_CODE};
use halogen_utils::verrors;

pub async fn handle(
    dbc: &DatabaseConnection,
    params: &UserShowParams,
) -> Result<UserData, ValidationErrors> {
    params.validate()?;

    let user_id = params.id.ok_or_else(|| {
        verrors(
            VALIDATION_ID_FIELD,
            VALIDATION_INVALID_CODE,
            "User ID is required".to_string(),
        )
    })?;

    let user = UserEntity::by_id_or_err(dbc, user_id).await?;

    info!("Fetched user '{}'", user.username);
    Ok(UserData::from(user))
}
