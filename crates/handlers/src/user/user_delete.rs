use halogen_orm::user::Entity as UserEntity;
use halogen_orm::{podcast, user, user_podcast};
use halogen_wire::{UserDeleteParams, ValidationErrors};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set,
    TransactionTrait,
};
use tracing::info;
use validator::Validate;

use crate::{db_error, not_found};

pub async fn handle(
    dbc: &DatabaseConnection,
    params: &UserDeleteParams,
) -> Result<(), ValidationErrors> {
    params.validate()?;

    let user_id = params.id;

    let txn = dbc
        .begin()
        .await
        .map_err(db_error("beginning user deletion"))?;

    let user = UserEntity::find()
        .filter(user::Column::Id.eq(user_id))
        .one(&txn)
        .await
        .map_err(db_error("fetching user for deletion"))?
        .ok_or_else(|| not_found("User not found"))?;

    // Transfer owned podcasts with other subscribers before deleting the user. Otherwise ON DELETE CASCADE would remove
    // shared episodes and every subscriber's playback/playlist history. Only unsubscribed podcasts should cascade.
    let owned = podcast::Entity::find()
        .filter(podcast::Column::OwnerId.eq(user_id))
        .all(&txn)
        .await
        .map_err(db_error("fetching owned podcasts"))?;
    for p in owned {
        let successor = user_podcast::Entity::find()
            .filter(user_podcast::Column::PodcastId.eq(p.id))
            .filter(user_podcast::Column::UserId.ne(user_id))
            .one(&txn)
            .await
            .map_err(db_error("finding a podcast successor owner"))?;
        if let Some(sub) = successor {
            let mut active: podcast::ActiveModel = p.into();
            active.owner_id = Set(sub.user_id);
            active
                .update(&txn)
                .await
                .map_err(db_error("reassigning podcast owner"))?;
        }
    }

    UserEntity::delete_many()
        .filter(user::Column::Id.eq(user_id))
        .exec(&txn)
        .await
        .map_err(db_error("deleting user"))?;

    txn.commit()
        .await
        .map_err(db_error("committing user deletion"))?;

    info!("Deleted user '{}'", user.username);
    Ok(())
}
