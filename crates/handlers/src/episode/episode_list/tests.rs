use super::handle;
use halogen_fixture::test_support::TestRoot;
use halogen_migrations::connect_and_migrate;
use halogen_orm::{episode, podcast, user, user_podcast};
use halogen_wire::{DefaultListParams, DownloadStatus, EpisodeInclude};
use sea_orm::{
    ActiveModelTrait,
    ActiveValue::{NotSet, Set},
    DatabaseConnection,
};

async fn seed_user(dbc: &DatabaseConnection, id: i32, username: &str) {
    user::ActiveModel {
        id: Set(id),
        username: Set(username.to_string()),
        password_hash: Set("x".to_string()),
        is_admin: Set(false),
        created_at: Set(chrono::Utc::now()),
        updated_at: Set(chrono::Utc::now()),
    }
    .insert(dbc)
    .await
    .expect("seed user");
}

/// Insert a podcast owned by `owner_id` and return its id.
async fn seed_podcast(dbc: &DatabaseConnection, owner_id: i32, title: &str) -> i32 {
    podcast::ActiveModel {
        id: NotSet,
        title: Set(title.to_string()),
        description: Set(String::new()),
        feed_url: Set(format!("https://feeds.example.com/{title}")),
        art_url: Set(None),
        author: Set(None),
        polled_at: Set(None),
        podcast_config_id: Set(None),
        owner_id: Set(owner_id),
        art_file_path: Set(None),
        etag: Set(None),
        last_modified: Set(None),
        feed_url_redirects: Set(None),
        created_at: Set(chrono::Utc::now()),
        updated_at: Set(chrono::Utc::now()),
    }
    .insert(dbc)
    .await
    .expect("seed podcast")
    .id
}

async fn seed_episode(dbc: &DatabaseConnection, podcast_id: i32, title: &str) -> i32 {
    episode::ActiveModel {
        id: NotSet,
        podcast_id: Set(podcast_id),
        title: Set(title.to_string()),
        description: Set(String::new()),
        content_url: Set(format!("https://audio.example.com/{title}.mp3")),
        guid: Set(None),
        art_url: Set(None),
        published_at: Set(None),
        downloaded_at: Set(None),
        content_file_path: Set(None),
        download_size: Set(None),
        art_file_path: Set(None),
        download_status: Set(DownloadStatus::NotDownloaded),
        download_started_at: Set(None),
        download_attempts: Set(0),
        duration_secs: Set(None),
        created_at: Set(chrono::Utc::now()),
        updated_at: Set(chrono::Utc::now()),
    }
    .insert(dbc)
    .await
    .expect("seed episode")
    .id
}

async fn subscribe(dbc: &DatabaseConnection, user_id: i32, podcast_id: i32) {
    user_podcast::ActiveModel {
        user_id: Set(user_id),
        podcast_id: Set(podcast_id),
        created_at: Set(chrono::Utc::now()),
        updated_at: Set(chrono::Utc::now()),
    }
    .insert(dbc)
    .await
    .expect("subscribe");
}

async fn seed_playback(dbc: &DatabaseConnection, user_id: i32, episode_id: i32, cursor: i64) {
    use halogen_orm::playback;
    playback::ActiveModel {
        id: NotSet,
        user_id: Set(user_id),
        episode_id: Set(episode_id),
        cursor: Set(cursor),
        completed: Set(false),
        created_at: Set(chrono::Utc::now()),
        updated_at: Set(chrono::Utc::now()),
    }
    .insert(dbc)
    .await
    .expect("seed playback");
}

/// `EpisodeInclude::Playback` embeds the **caller's** resume cursor and only
/// the caller's: alice sees her cursor, bob (same episode, no row) sees `None`,
/// and omitting the include leaves `playback` `None` for everyone.
#[tokio::test]
async fn playback_include_embeds_only_callers_cursor() {
    let mut root = TestRoot::new("episode_list_playback_include");
    let db_path = root.path().join("halogen.db");
    let dbc = connect_and_migrate(&db_path, true).await.unwrap();

    seed_user(&dbc, 1, "alice").await;
    seed_user(&dbc, 2, "bob").await;
    let pod = seed_podcast(&dbc, 1, "pod").await;
    let ep = seed_episode(&dbc, pod, "ep").await;
    subscribe(&dbc, 1, pod).await;
    subscribe(&dbc, 2, pod).await;
    // Only alice (user 1) has a saved position.
    seed_playback(&dbc, 1, ep, 123).await;

    let with_pb = DefaultListParams::<EpisodeInclude> {
        includes: Some(vec![EpisodeInclude::Playback]),
        ..Default::default()
    };

    // Alice: cursor embedded.
    let (alice_eps, _) = handle(&dbc, 1, &with_pb).await.unwrap();
    assert_eq!(
        alice_eps[0].playback.as_ref().map(|p| p.cursor),
        Some(123),
        "caller's cursor is embedded with the Playback include"
    );

    // Bob: same episode, no row of his own → no leak of alice's cursor.
    let (bob_eps, _) = handle(&dbc, 2, &with_pb).await.unwrap();
    assert!(
        bob_eps[0].playback.is_none(),
        "another user's cursor must never be embedded"
    );

    // No include → never populated, even for alice.
    let no_pb = DefaultListParams::<EpisodeInclude>::default();
    let (alice_no_inc, _) = handle(&dbc, 1, &no_pb).await.unwrap();
    assert!(
        alice_no_inc[0].playback.is_none(),
        "playback stays None unless the include is requested"
    );

    drop(dbc);
    root.mark_success();
}

/// `GET /episodes` must only return episodes from podcasts the caller is
/// subscribed to — never another owner's, and never a non-subscribed
/// podcast's, even when both live in the same database.
#[tokio::test]
async fn list_episodes_scoped_to_subscriptions() {
    let mut root = TestRoot::new("episode_list_scope");
    let db_path = root.path().join("halogen.db");
    let dbc = connect_and_migrate(&db_path, true).await.unwrap();

    // Two users; `alice` (id 1) is our caller, `bob` (id 2) owns a rival podcast.
    seed_user(&dbc, 1, "alice").await;
    seed_user(&dbc, 2, "bob").await;

    // Alice owns + is subscribed to `subbed`; she also owns `unsubbed` but is
    // NOT subscribed to it. Bob owns `bobcast`, which Alice can't see at all.
    let subbed = seed_podcast(&dbc, 1, "subbed").await;
    let unsubbed = seed_podcast(&dbc, 1, "unsubbed").await;
    let bobcast = seed_podcast(&dbc, 2, "bobcast").await;

    let visible = seed_episode(&dbc, subbed, "visible-ep").await;
    let _hidden_unsubbed = seed_episode(&dbc, unsubbed, "hidden-unsubbed-ep").await;
    let _hidden_bob = seed_episode(&dbc, bobcast, "hidden-bob-ep").await;

    subscribe(&dbc, 1, subbed).await;

    let params = DefaultListParams::<EpisodeInclude>::default();
    let (episodes, paginator) = handle(&dbc, 1, &params).await.unwrap();

    let ids: Vec<i32> = episodes.iter().map(|e| e.id).collect();
    assert_eq!(ids, vec![visible], "only the subscribed podcast's episode");
    assert_eq!(paginator.total, 1, "paginator total must reflect the scope");

    // A user with no subscriptions sees nothing — `is_in([])` returns no rows.
    let (none, none_paginator) = handle(&dbc, 2, &params).await.unwrap();
    assert!(none.is_empty(), "bob is subscribed to nothing");
    assert_eq!(none_paginator.total, 0);

    drop(dbc);
    root.mark_success();
}
