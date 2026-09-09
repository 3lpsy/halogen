#[cfg(test)]
mod tests {
    use crate::tests::harness::{episode_model, new_test_db, podcast_model, subscribe, user_model};
    use halogen_handlers::user::user_delete::handle;
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
