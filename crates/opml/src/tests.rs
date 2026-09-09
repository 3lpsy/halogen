use std::fs;
use std::path::Path;

use halogen_migrations::connect_and_migrate;
use halogen_orm::podcast::Column;
use halogen_orm::podcast::Entity as PodcastEntity;
use halogen_utils::opml::ImportPodcastResult;
use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter};

use crate::import_podcasts_from_opml;
use halogen_fixture::test_support::TestRoot;

/// Seed a user with id=1 so imported podcasts (which use `owner_id = 1`) satisfy
/// the enforced `podcast.owner_id → user` foreign key.
async fn seed_owner(dbc: &sea_orm::DatabaseConnection) {
    use halogen_orm::user::ActiveModel;
    use sea_orm::{ActiveModelTrait, ActiveValue::Set};
    ActiveModel {
        id: Set(1),
        username: Set("owner".to_string()),
        password_hash: Set("x".to_string()),
        is_admin: Set(false),
        created_at: Set(chrono::Utc::now()),
        updated_at: Set(chrono::Utc::now()),
    }
    .insert(dbc)
    .await
    .expect("seed owner user");
}

async fn import_opml_file_and_assert(opml_path: &str) -> ImportPodcastResult {
    let mut root = TestRoot::new("opml_import");
    let db_path = root.path().join("halogen.db");

    let dbc = connect_and_migrate(&db_path, true)
        .await
        .expect("Failed to connect and migrate");
    seed_owner(&dbc).await;
    let dbc_clone = dbc.clone();

    let opml_file = Path::new(opml_path);
    let result = import_podcasts_from_opml(&dbc, opml_file, 1)
        .await
        .expect("OPML import failed");

    assert!(result.created > 0, "Should have created podcasts");

    let podcast_count = PodcastEntity::find().count(&dbc_clone).await.unwrap();

    assert_eq!(
        podcast_count as usize, result.total,
        "Podcast count should match import result"
    );

    drop(dbc);
    root.mark_success();
    result
}

#[tokio::test]
async fn test_opml_import_creates_podcast_entries() {
    let opml_path = "/tmp/halogen_test_opml.opml";

    let opml_content = r#"<?xml version="1.0"?>
    <opml version="1.0">
      <head>
        <title>Test OPML</title>
      </head>
      <body>
        <outline text="Test Podcast 1" type="rss" xmlUrl="https://feeds.example.com/test1"/>
        <outline text="Test Podcast 2" type="rss" xmlUrl="https://feeds.example.com/test2"/>
        <outline text="Test Podcast 3" type="rss" xmlUrl="https://feeds.example.com/test3"/>
      </body>
    </opml>
    "#;

    fs::write(opml_path, opml_content).expect("Failed to write test OPML file");

    let result = import_opml_file_and_assert(opml_path).await;

    assert_eq!(result.created, 3, "Should have created 3 podcasts");
    assert_eq!(result.total, 3, "Should have processed 3 podcasts");
    assert_eq!(result.podcast_names.len(), 3, "Should have 3 podcast names");
}

#[tokio::test]
async fn test_opml_import_skips_existing_podcasts() {
    let opml_path = "/tmp/halogen_test_opml_skip.opml";

    let opml_content = r#"<?xml version="1.0"?>
    <opml version="1.0">
      <head>
        <title>Test OPML</title>
      </head>
      <body>
        <outline text="Existing Podcast" type="rss" xmlUrl="https://feeds.example.com/existing"/>
        <outline text="New Podcast" type="rss" xmlUrl="https://feeds.example.com/new"/>
      </body>
    </opml>
    "#;

    fs::write(opml_path, opml_content).expect("Failed to write test OPML file");

    let mut root = TestRoot::new("opml_skip");
    let db_path = root.path().join("halogen.db");

    let dbc = connect_and_migrate(&db_path, true)
        .await
        .expect("Failed to connect and migrate");
    seed_owner(&dbc).await;

    let dbc_clone = dbc.clone();

    let opml_file = Path::new(opml_path);
    let result = import_podcasts_from_opml(&dbc, opml_file, 1).await.unwrap();

    assert_eq!(result.created, 2, "Should create 2 new podcasts");

    let podcast = PodcastEntity::find()
        .filter(Column::FeedUrl.eq("https://feeds.example.com/existing"))
        .one(&dbc_clone)
        .await
        .expect("Existing podcast should exist")
        .expect("podcast should exist");

    assert_eq!(podcast.title, "Existing Podcast");

    drop(dbc);
    root.mark_success();
}

#[tokio::test]
async fn test_opml_import_with_varied_content_types() {
    let opml_path = "/tmp/halogen_test_opml_types.opml";

    let opml_content = r#"<?xml version="1.0"?>
    <opml version="1.0">
      <head>
        <title>Test OPML</title>
      </head>
      <body>
        <outline text="RSS Podcast 1" type="rss" xmlUrl="https://feeds.example.com/rss1"/>
        <outline text="Regular Text" type="" xmlUrl="https://example.com"/>
        <outline text="RSS Podcast 2" type="rss" xmlUrl="https://feeds.example.com/rss2"/>
        <outline text="YouTube Channel" type="youtube" xmlUrl="https://youtube.com/xyz"/>
        <outline text="RSS Podcast 3" type="rss" xmlUrl="https://feeds.example.com/rss3"/>
        <outline text="Nested Podcast" type="outline" xmlUrl="https://feeds.example.com/nested"/>
      </body>
    </opml>
    "#;

    fs::write(opml_path, opml_content).expect("Failed to write test OPML file");

    let mut root = TestRoot::new("opml_test_types");
    let db_path = root.path().join("halogen.db");

    let dbc = connect_and_migrate(&db_path, true)
        .await
        .expect("Failed to connect and migrate");
    seed_owner(&dbc).await;
    let dbc_clone = dbc.clone();

    let opml_file = Path::new(opml_path);
    let result = import_podcasts_from_opml(&dbc, opml_file, 1).await.unwrap();

    assert_eq!(result.created, 3, "Should create 3 RSS podcasts");

    let podcast_count = PodcastEntity::find().count(&dbc_clone).await.unwrap();

    assert_eq!(podcast_count, 3, "Database should have 3 podcasts");

    drop(dbc);
    root.mark_success();
}

#[tokio::test]
async fn test_opml_import_empty_feed_urls() {
    let opml_path = "/tmp/halogen_test_opml_empty.opml";

    let opml_content = r#"<?xml version="1.0"?>
    <opml version="1.0">
      <head>
        <title>Test OPML</title>
      </head>
      <body>
        <outline text="RSS Podcast Good" type="rss" xmlUrl="https://feeds.example.com/good"/>
        <outline text="RSS Podcast No URL" type="rss" xmlUrl=""/>
        <outline text="RSS Podcast Another Good" type="rss" xmlUrl="https://feeds.example.com/another"/>
      </body>
    </opml>
    "#;

    fs::write(opml_path, opml_content).expect("Failed to write test OPML file");

    let mut root = TestRoot::new("opml_empty");
    let db_path = root.path().join("halogen.db");

    let dbc = connect_and_migrate(&db_path, true)
        .await
        .expect("Failed to connect and migrate");
    seed_owner(&dbc).await;

    let dbc_clone = dbc.clone();

    let opml_file = Path::new(opml_path);
    let result = import_podcasts_from_opml(&dbc, opml_file, 1).await.unwrap();

    assert_eq!(
        result.created, 2,
        "Should create 2 podcasts (skip one with empty URL)"
    );
    assert_eq!(result.skipped, 1, "Should skip 1 podcast for missing URL");

    let podcast_count = PodcastEntity::find().count(&dbc_clone).await.unwrap();

    assert_eq!(podcast_count, 2, "Database should have 2 podcasts");

    drop(dbc);
    root.mark_success();
}

#[tokio::test]
async fn test_opml_import_duplicate_feed_urls() {
    let opml_path = "/tmp/halogen_test_opml_duplicates.opml";

    let opml_content = r#"<?xml version="1.0"?>
    <opml version="1.0">
      <head>
        <title>Test OPML</title>
      </head>
      <body>
        <outline text="Same Feed Twice" type="rss" xmlUrl="https://feeds.example.com/duplicate"/>
        <outline text="Another Podcast" type="rss" xmlUrl="https://feeds.example.com/duplicate"/>
      </body>
    </opml>
    "#;

    fs::write(opml_path, opml_content).expect("Failed to write test OPML file");

    let mut root = TestRoot::new("opml_duplicates");
    let db_path = root.path().join("halogen.db");

    let dbc = connect_and_migrate(&db_path, true)
        .await
        .expect("Failed to connect and migrate");
    seed_owner(&dbc).await;

    let dbc_clone = dbc.clone();

    let opml_file = Path::new(opml_path);
    let result = import_podcasts_from_opml(&dbc, opml_file, 1).await.unwrap();

    assert_eq!(
        result.created, 1,
        "Should create 1 podcast (duplicate should be skipped)"
    );
    assert_eq!(result.skipped, 1, "Should skip 1 duplicate");

    let podcast_count = PodcastEntity::find().count(&dbc_clone).await.unwrap();

    assert_eq!(podcast_count, 1, "Should have only 1 podcast");

    drop(dbc);
    root.mark_success();
}

/// The owner must be subscribed (`user_podcast`) to every imported feed, including
/// on a re-import where the podcast already exists. Without the subscription the
/// imported library is owned-but-invisible: `GET /episodes` and `GET /podcasts`
/// scope to subscriptions, not ownership, so the lists come back empty.
#[tokio::test]
async fn test_opml_import_subscribes_owner() {
    use halogen_orm::user_podcast;

    let opml_path = "/tmp/halogen_test_opml_subscribe.opml";
    let opml_content = r#"<?xml version="1.0"?>
    <opml version="1.0">
      <head>
        <title>Test OPML</title>
      </head>
      <body>
        <outline text="Sub Podcast 1" type="rss" xmlUrl="https://feeds.example.com/sub1"/>
        <outline text="Sub Podcast 2" type="rss" xmlUrl="https://feeds.example.com/sub2"/>
      </body>
    </opml>
    "#;
    fs::write(opml_path, opml_content).expect("Failed to write test OPML file");

    let mut root = TestRoot::new("opml_subscribe");
    let db_path = root.path().join("halogen.db");
    let dbc = connect_and_migrate(&db_path, true)
        .await
        .expect("Failed to connect and migrate");
    seed_owner(&dbc).await;

    let opml_file = Path::new(opml_path);
    import_podcasts_from_opml(&dbc, opml_file, 1)
        .await
        .expect("OPML import failed");

    let subs = user_podcast::Entity::find()
        .filter(user_podcast::Column::UserId.eq(1))
        .count(&dbc)
        .await
        .unwrap();
    assert_eq!(subs, 2, "owner should be subscribed to both imported feeds");

    // Re-import the same file: podcasts are skipped, but the owner must remain
    // subscribed exactly once each — no duplicate-key error, no extra rows.
    import_podcasts_from_opml(&dbc, opml_file, 1)
        .await
        .expect("OPML re-import failed");
    let subs_after = user_podcast::Entity::find()
        .filter(user_podcast::Column::UserId.eq(1))
        .count(&dbc)
        .await
        .unwrap();
    assert_eq!(subs_after, 2, "re-import is idempotent for subscriptions");

    drop(dbc);
    root.mark_success();
}
