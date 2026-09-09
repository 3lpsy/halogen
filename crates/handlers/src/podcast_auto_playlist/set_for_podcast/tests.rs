use super::*;
use halogen_fixture::test_support::TestRoot;
use halogen_migrations::connect_and_migrate;
use halogen_orm::{playlist, podcast, user};
use sea_orm::ActiveModelTrait;

async fn mk_user(dbc: &sea_orm::DatabaseConnection, id: i32, name: &str) {
    let now = chrono::Utc::now();
    user::ActiveModel {
        id: Set(id),
        username: Set(name.to_string()),
        password_hash: Set("x".to_string()),
        is_admin: Set(false),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(dbc)
    .await
    .expect("insert user");
}

async fn mk_podcast(dbc: &sea_orm::DatabaseConnection, id: i32, owner_id: i32) {
    let now = chrono::Utc::now();
    podcast::ActiveModel {
        id: Set(id),
        title: Set("P".to_string()),
        description: Set(String::new()),
        feed_url: Set(format!("https://feed.test/{id}")),
        art_url: Set(None),
        author: Set(None),
        polled_at: Set(None),
        podcast_config_id: Set(None),
        owner_id: Set(owner_id),
        art_file_path: Set(None),
        etag: Set(None),
        last_modified: Set(None),
        feed_url_redirects: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(dbc)
    .await
    .expect("insert podcast");
}

async fn mk_playlist(dbc: &sea_orm::DatabaseConnection, id: i32, user_id: i32) {
    let now = chrono::Utc::now();
    playlist::ActiveModel {
        id: Set(id),
        name: Set(format!("pl{id}")),
        description: Set(None),
        user_id: Set(user_id),
        is_default: Set(false),
        position: Set(0),
        on_remove_delete_file_server: Set(false),
        on_remove_delete_file_client: Set(false),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(dbc)
    .await
    .expect("insert playlist");
}

/// Regression (H2 IDOR): `set_for_podcast` may only link playlists owned by the
/// PODCAST'S OWNER. A playlist owned by another user must be silently dropped, so
/// the poller can never inject episodes into a victim's playlist.
#[tokio::test]
async fn set_for_podcast_drops_foreign_owned_playlists() {
    let mut root = TestRoot::new("auto_playlist_owner_scope");
    let db_path = root.path().join("halogen.db");
    let dbc = connect_and_migrate(&db_path, true).await.expect("test db");

    mk_user(&dbc, 1, "owner").await;
    mk_user(&dbc, 2, "victim").await;
    mk_podcast(&dbc, 10, 1).await; // podcast owned by user 1
    mk_playlist(&dbc, 100, 1).await; // owned by the podcast owner
    mk_playlist(&dbc, 200, 2).await; // owned by a DIFFERENT user (victim)

    let data = PodcastAutoPlaylistSetData {
        playlist_ids: vec![100, 200],
        add_to_start: None,
    };
    let kept = handle(&dbc, 10, data).await.expect("set auto-playlists");

    // Only the owner's playlist survives; the victim's (200) is dropped.
    let ids: Vec<i32> = kept.iter().map(|r| r.playlist_id).collect();
    assert_eq!(ids, vec![100], "foreign-owned playlist must be dropped");

    // And no auto-playlist row references the foreign playlist.
    let rows = PapEntity::find().all(&dbc).await.unwrap();
    assert!(
        rows.iter().all(|r| r.playlist_id != 200),
        "no link to a playlist owned by another user"
    );

    let _ = dbc.close().await;
    root.mark_success();
}
