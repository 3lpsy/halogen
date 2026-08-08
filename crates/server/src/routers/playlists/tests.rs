use halogen_orm::episode_playlist::ActiveModel as EpisodePlaylistActiveModel;
use sea_orm::ActiveModelTrait;
use sea_orm::ActiveValue::Set;

use crate::tests::harness::{episode_model, playlist_model, podcast_model, seed_admin_and_user};
use halogen_fixture::test_support::TestRoot;

pub use crate::tests::harness::{build_test_router, generate_jwt_token};

/// Create a test SQLite database with seeded playlists and episode-playlists using SeaORM entities.
pub async fn setup_test_db() -> (TestRoot, sea_orm::DatabaseConnection, serde_json::Value) {
    let (mut root, dbc) = crate::tests::harness::new_test_db("test_playlists").await;

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

    // Two playlists owned by the acting user so ownership guards pass.
    let playlist_id_1 = i32::MAX - 5;
    let mut playlist_1 = playlist_model(playlist_id_1, user_id);
    playlist_1.name = Set("Test Playlist 1".to_string());
    playlist_1.description = Set(Some("Playlist 1 description".to_string()));
    playlist_1.insert(&dbc).await.expect("insert playlist 1");

    let playlist_id_2 = i32::MAX - 6;
    let mut playlist_2 = playlist_model(playlist_id_2, user_id);
    playlist_2.name = Set("Test Playlist 2".to_string());
    playlist_2.description = Set(Some("Playlist 2 description".to_string()));
    playlist_2.position = Set(1);
    playlist_2.insert(&dbc).await.expect("insert playlist 2");

    // Episode 1 belongs to playlist 1.
    EpisodePlaylistActiveModel {
        episode_id: Set(episode_id_1),
        playlist_id: Set(playlist_id_1),
        position: Set(0),
        created_at: Set(chrono::Utc::now()),
        updated_at: Set(chrono::Utc::now()),
    }
    .insert(&dbc)
    .await
    .expect("insert episode-playlist");

    let payload = serde_json::json!({
        "admin_id": admin_id.to_string(),
        "admin_password": "testadmin123",
        "user_id": user_id.to_string(),
        "user_password": "testuser123",
        "podcast_id": podcast_id.to_string(),
        "episode_id_1": episode_id_1.to_string(),
        "episode_id_2": episode_id_2.to_string(),
        "episode_id": episode_id_1.to_string(),
        "playlist_id_1": playlist_id_1.to_string(),
        "playlist_id_2": playlist_id_2.to_string(),
        "playlist_id": playlist_id_1.to_string(),
    });

    root.mark_success();
    (root, dbc, payload)
}
