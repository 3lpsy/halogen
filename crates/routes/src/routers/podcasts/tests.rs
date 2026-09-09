use sea_orm::ActiveModelTrait;
use sea_orm::ActiveValue::Set;

use crate::tests::harness::{podcast_config_model, podcast_model, seed_admin_and_user, subscribe};
use halogen_fixture::test_support::TestRoot;

pub use crate::tests::harness::{build_test_router, generate_jwt_token};

/// Create a test SQLite database with seeded podcasts using SeaORM entities.
pub async fn setup_test_db() -> (TestRoot, sea_orm::DatabaseConnection, serde_json::Value) {
    let (mut root, dbc) = crate::tests::harness::new_test_db("test_podcast").await;

    let admin_id = i32::MAX;
    let user_id = i32::MAX - 1;
    seed_admin_and_user(&dbc, admin_id, user_id).await;

    // First podcast — the factory defaults already match ("Test Podcast", etc.).
    let podcast_id = i32::MAX - 2;
    podcast_model(podcast_id, user_id)
        .insert(&dbc)
        .await
        .expect("insert podcast");

    // Second podcast — distinct title/feed and a non-null art_url.
    let podcast_id_2 = i32::MAX - 3;
    let mut podcast_2 = podcast_model(podcast_id_2, user_id);
    podcast_2.title = Set("Second Podcast".to_string());
    podcast_2.description = Set("Another podcast".to_string());
    podcast_2.feed_url = Set("https://example.com/feed2.xml".to_string());
    podcast_2.art_url = Set(Some("Artist Name".to_string()));
    podcast_2.insert(&dbc).await.expect("insert podcast 2");

    // Subscribe the regular user to both podcasts so the subscription-scoped
    // list/get returns them. The user also owns them (owner_id above), so the
    // ownership-guarded write routes (update/delete/config) succeed too.
    for pid in [podcast_id, podcast_id_2] {
        subscribe(&dbc, user_id, pid).await;
    }

    let podcast_config_id = i32::MAX - 4;
    podcast_config_model(podcast_config_id)
        .insert(&dbc)
        .await
        .expect("insert podcast config");

    let payload = serde_json::json!({
        "admin_id": admin_id.to_string(),
        "admin_password": "testadmin123",
        "user_id": user_id.to_string(),
        "user_password": "testuser123",
        "podcast_id": podcast_id.to_string(),
        "podcast_id_2": podcast_id_2.to_string(),
        "podcast_config_id": podcast_config_id.to_string(),
    });

    root.mark_success();
    (root, dbc, payload)
}
