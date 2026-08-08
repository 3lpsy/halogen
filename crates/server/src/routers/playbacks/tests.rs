use sea_orm::ActiveModelTrait;
use sea_orm::ActiveValue::Set;

use crate::tests::harness::{episode_model, podcast_model, seed_admin_and_user};
use halogen_fixture::test_support::TestRoot;

pub use crate::tests::harness::{build_test_router, generate_jwt_token};

/// Create a test SQLite database with seeded playback data using SeaORM entities.
pub async fn setup_test_db() -> (TestRoot, sea_orm::DatabaseConnection, serde_json::Value) {
    let (mut root, dbc) = crate::tests::harness::new_test_db("test_playback").await;

    let admin_id = i32::MAX;
    let user_id = i32::MAX - 1;
    seed_admin_and_user(&dbc, admin_id, user_id).await;

    let podcast_id = i32::MAX - 2;
    podcast_model(podcast_id, user_id)
        .insert(&dbc)
        .await
        .expect("insert podcast");

    let episode_id_1 = i32::MAX - 3;
    let mut episode_1 = episode_model(episode_id_1, podcast_id);
    episode_1.title = Set("Test Episode 1".to_string());
    episode_1.description = Set("Episode 1 description".to_string());
    episode_1.content_url = Set("https://example.com/ep1.mp3".to_string());
    episode_1.insert(&dbc).await.expect("insert episode 1");

    let episode_id_2 = i32::MAX - 4;
    let mut episode_2 = episode_model(episode_id_2, podcast_id);
    episode_2.title = Set("Test Episode 2".to_string());
    episode_2.description = Set("Episode 2 description".to_string());
    episode_2.content_url = Set("https://example.com/ep2.mp3".to_string());
    episode_2.insert(&dbc).await.expect("insert episode 2");

    let payload = serde_json::json!({
        "admin_id": admin_id.to_string(),
        "admin_password": "testadmin123",
        "user_id": user_id.to_string(),
        "user_password": "testuser123",
        "podcast_id": podcast_id.to_string(),
        "episode_id_1": episode_id_1.to_string(),
        "episode_id_2": episode_id_2.to_string(),
        "episode_id": episode_id_1.to_string(),
    });

    root.mark_success();
    (root, dbc, payload)
}
