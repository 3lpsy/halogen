use halogen_orm::user::Entity as UserEntity;
use halogen_orm::{podcast, user, user_podcast};
use halogen_wire::{UserDeleteParams, ValidationErrors};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set,
    TransactionTrait,
};
use tracing::info;
use validator::Validate;

use crate::handlers::{db_error, not_found};

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

    // Podcasts are SHARED rows owned by whoever first added the feed, and the owner
    // FK is `ON DELETE CASCADE`. Deleting a user would therefore cascade away every
    // podcast they own — and its episodes, and EVERY other subscriber's playlist
    // entries, playbacks, and listen history for it. So first hand off any owned
    // podcast that still has another subscriber to that subscriber; only podcasts
    // with no remaining subscribers fall to the cascade.
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

#[cfg(test)]
mod tests {
    use super::handle;
    use crate::tests::harness::{episode_model, new_test_db, podcast_model, subscribe, user_model};
    use halogen_orm::{episode, podcast};
    use halogen_wire::UserDeleteParams;
    use sea_orm::{ActiveModelTrait, EntityTrait};

    // Deleting the owner of a podcast that ANOTHER user is subscribed to must hand
    // the podcast off to that subscriber, not cascade it (and its episodes, and the
    // other user's data) away.
    #[tokio::test]
    async fn deleting_shared_podcast_owner_reassigns_to_subscriber() {
        let (_root, dbc) = new_test_db("user_delete_reassign").await;

        user_model(1, "alice", "pw", false)
            .insert(&dbc)
            .await
            .unwrap();
        user_model(2, "bob", "pw", false)
            .insert(&dbc)
            .await
            .unwrap();
        podcast_model(10, 1).insert(&dbc).await.unwrap(); // owned by alice
        subscribe(&dbc, 1, 10).await;
        subscribe(&dbc, 2, 10).await;
        episode_model(100, 10).insert(&dbc).await.unwrap();

        handle(&dbc, &UserDeleteParams { id: 1 })
            .await
            .expect("delete owner");

        let pod = podcast::Entity::find_by_id(10)
            .one(&dbc)
            .await
            .unwrap()
            .expect("shared podcast must survive the owner's deletion");
        assert_eq!(
            pod.owner_id, 2,
            "podcast must be reassigned to the remaining subscriber"
        );
        assert!(
            episode::Entity::find_by_id(100)
                .one(&dbc)
                .await
                .unwrap()
                .is_some(),
            "the podcast's episodes must survive too"
        );
    }
}
