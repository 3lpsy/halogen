use super::*;
use halogen_fixture::test_support::TestRoot;
use halogen_fixture::test_support::load_fixture;
use halogen_migrations::connect_and_migrate;
use halogen_orm::episode::{ActiveModel as EpisodeActiveModel, Entity as EpisodeEntity};
use halogen_orm::podcast::{ActiveModel as PodcastActiveModel, Entity as PodcastEntity};
use halogen_orm::user::ActiveModel as UserActiveModel;
use halogen_rss as rss;
use halogen_wire::DownloadStatus;
use sea_orm::ActiveModelTrait;
use sea_orm::EntityTrait;
use sea_orm::Set;
use tokio::time::sleep;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn create_admin_user(dbc: &DatabaseConnection) -> i32 {
    let admin_id = 1;
    let admin_hashed =
        bcrypt::hash("testadmin123", bcrypt::DEFAULT_COST).expect("hash admin password");

    let admin = UserActiveModel {
        id: Set(admin_id),
        username: Set("admin_user".to_string()),
        password_hash: Set(admin_hashed),
        is_admin: Set(true),
        created_at: Set(chrono::Utc::now()),
        updated_at: Set(chrono::Utc::now()),
    };

    admin.insert(dbc).await.expect("insert admin");
    admin_id
}

async fn create_podcast(dbc: &DatabaseConnection, id: i32, feed_url: &str) -> i32 {
    let now = chrono::Utc::now();
    let podcast = PodcastActiveModel {
        id: Set(id),
        title: Set("Test Podcast".to_string()),
        description: Set("Podcast description".to_string()),
        feed_url: Set(feed_url.to_string()),
        art_url: Set(None),
        art_file_path: Set(None),
        author: Set(None),
        etag: Set(None),
        last_modified: Set(None),
        polled_at: Set(None),
        podcast_config_id: Set(None),
        owner_id: Set(1),
        feed_url_redirects: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
    };

    podcast.insert(dbc).await.expect("insert podcast");
    id
}

async fn create_episode(
    dbc: &DatabaseConnection,
    podcast_id: i32,
    content_url: &str,
    title: &str,
) -> i32 {
    let episode_id = 100;
    let now = chrono::Utc::now();
    let episode = EpisodeActiveModel {
        id: Set(episode_id),
        podcast_id: Set(podcast_id),
        title: Set(title.to_string()),
        description: Set("Episode description".to_string()),
        content_url: Set(content_url.to_string()),
        guid: Set(None),
        art_url: Set(None),
        published_at: Set(Some(chrono::Utc::now())),
        downloaded_at: Set(None),
        content_file_path: Set(None),
        download_size: Set(None),
        art_file_path: Set(None),
        download_status: Set(DownloadStatus::NotDownloaded),
        download_started_at: Set(None),
        download_attempts: Set(0),
        duration_secs: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
    };

    episode.insert(dbc).await.expect("insert episode");
    episode_id
}

#[tokio::test]
async fn test_handle_start_stop() {
    let mut root = TestRoot::new("polling_start_stop");
    let db_path = root.path().join("halogen.db");
    let dbc = connect_and_migrate(&db_path, true)
        .await
        .expect("create test db");
    create_admin_user(&dbc).await;

    let handle = PollingHandle::new(dbc.clone(), Duration::from_secs(60), 5);

    assert!(!handle.is_running());
    handle.start().expect("start polling");
    assert!(handle.is_running());
    assert_eq!(
        handle.start().unwrap_err(),
        "Polling service is already running"
    );
    handle.stop().expect("stop polling");
    assert!(!handle.is_running());
    assert_eq!(handle.stop().unwrap_err(), "Polling service is not running");

    let _ = dbc.close().await;
    root.mark_success();
}

/// In-process hosts must await task termination before dropping their DB pool; shutdown is idempotent.
#[tokio::test]
async fn test_handle_shutdown_terminates_and_is_idempotent() {
    let mut root = TestRoot::new("polling_shutdown");
    let db_path = root.path().join("halogen.db");
    let dbc = connect_and_migrate(&db_path, true)
        .await
        .expect("create test db");
    create_admin_user(&dbc).await;

    let handle = PollingHandle::new(dbc.clone(), Duration::from_secs(60), 5);
    handle.start().expect("start polling");
    assert!(handle.is_running());

    handle.shutdown().await;
    assert!(!handle.is_running());
    // Idempotent on a stopped service (unlike `stop`, which errors).
    handle.shutdown().await;
    assert!(!handle.is_running());
    // The slot is free again — a fresh start works after a shutdown.
    handle.start().expect("restart polling");
    handle.shutdown().await;
    assert!(!handle.is_running());

    let _ = dbc.close().await;
    root.mark_success();
}

#[tokio::test]
async fn test_handle_zero_interval_does_not_panic() {
    let mut root = TestRoot::new("polling_zero_interval");
    let db_path = root.path().join("halogen.db");
    let dbc = connect_and_migrate(&db_path, true)
        .await
        .expect("create test db");
    create_admin_user(&dbc).await;

    let handle = PollingHandle::new(dbc.clone(), Duration::ZERO, 5);
    handle.start().expect("start zero-interval polling");
    assert!(handle.is_running());
    sleep(Duration::from_millis(200)).await;
    handle.stop().expect("stop zero-interval polling");
    assert!(!handle.is_running());

    let _ = dbc.close().await;
    root.mark_success();
}

#[tokio::test]
async fn test_manual_poll_inserts_new_episodes() {
    let mut root = TestRoot::new("polling_manual_poll");
    let db_path = root.path().join("halogen.db");
    let dbc = connect_and_migrate(&db_path, true)
        .await
        .expect("create test db");
    create_admin_user(&dbc).await;

    let mock_server = MockServer::start().await;

    let podcast_id = 1;
    create_podcast(&dbc, podcast_id, &mock_server.uri()).await;

    let handle = PollingHandle::new(dbc.clone(), Duration::ZERO, 5);

    let fixture = load_fixture("simplecast_the_daily.xml");
    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture))
        .mount(&mock_server)
        .await;

    handle.poll().await.expect("manual poll");

    let episodes = EpisodeEntity::find()
        .all(&dbc)
        .await
        .expect("fetch episodes");
    assert!(
        !episodes.is_empty(),
        "poll should have inserted episodes from RSS feed"
    );

    handle.stop().ok();
    let _ = dbc.close().await;
    root.mark_success();
}

#[tokio::test]
async fn test_manual_poll_skips_existing_episodes() {
    let mut root = TestRoot::new("polling_skip_existing");
    let db_path = root.path().join("halogen.db");
    let dbc = connect_and_migrate(&db_path, true)
        .await
        .expect("create test db");
    create_admin_user(&dbc).await;

    let mock_server = MockServer::start().await;

    let podcast_id = 1;
    create_podcast(&dbc, podcast_id, &mock_server.uri()).await;

    let existing_url = "https://dts.podtrac.com/redirect.mp3/pdst.fm/e/pfx.vpixl.com/6qj4J/pscrb.fm/rss/p/nyt.simplecastaudio.com/03d8b493-87fc-4bd1-931f-8a8e9b945d8a/episodes/bb5dcfa4-6c27-4bd2-a233-08a825d88abd/audio/128/default.mp3?aid=rss_feed";
    create_episode(&dbc, podcast_id, existing_url, "Existing Episode").await;

    let handle = PollingHandle::new(dbc.clone(), Duration::ZERO, 5);

    let fixture = load_fixture("simplecast_the_daily.xml");
    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture))
        .mount(&mock_server)
        .await;

    handle.poll().await.expect("manual poll");

    let episodes = EpisodeEntity::find()
        .all(&dbc)
        .await
        .expect("fetch episodes");
    assert!(
        !episodes.is_empty(),
        "poll should have inserted new episodes"
    );
    let existing_count = episodes
        .iter()
        .filter(|ep| ep.content_url == existing_url)
        .count();
    assert_eq!(
        existing_count, 1,
        "existing episode should not be duplicated"
    );

    handle.stop().ok();
    let _ = dbc.close().await;
    root.mark_success();
}

// Poll adopts channel-level art onto the podcast row and heals a known
// episode's stale `art_url` (pre-fix ingest stored the MP3 enclosure there).
#[tokio::test]
async fn test_poll_adopts_channel_art_and_heals_episode_art() {
    let mut root = TestRoot::new("polling_art_adopt_heal");
    let db_path = root.path().join("halogen.db");
    let dbc = connect_and_migrate(&db_path, true)
        .await
        .expect("create test db");
    create_admin_user(&dbc).await;

    let mock_server = MockServer::start().await;
    let podcast_id = 1;
    create_podcast(&dbc, podcast_id, &mock_server.uri()).await;

    // Known episode (matched via the fixture item's <guid> — enclosure URLs
    // carry rotating tracking params, so guid is the robust identity) with
    // the legacy bug: its art_url is an MP3 enclosure.
    let stale_mp3 = "https://origin.test/legacy-enclosure.mp3";
    let ep_id = create_episode(&dbc, podcast_id, "https://origin.test/e.mp3", "Existing").await;
    let stale = EpisodeActiveModel {
        id: Set(ep_id),
        // Guid of a real item in simplecast_the_daily.xml.
        guid: Set(Some("4e35eb28-fd35-4f2e-b595-418c464d923e".to_string())),
        art_url: Set(Some(stale_mp3.to_string())),
        ..Default::default()
    };
    stale.update(&dbc).await.expect("seed stale art_url");

    let handle = PollingHandle::new(dbc.clone(), Duration::ZERO, 5);
    let fixture = load_fixture("simplecast_the_daily.xml");
    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture))
        .mount(&mock_server)
        .await;
    handle.poll().await.expect("manual poll");

    // Podcast adopted the channel image.
    let p = PodcastEntity::find_by_id(podcast_id)
        .one(&dbc)
        .await
        .unwrap()
        .unwrap();
    let pod_art = p.art_url.expect("podcast art adopted from channel");
    assert!(
        pod_art.starts_with("https://"),
        "channel art URL: {pod_art}"
    );
    assert!(!pod_art.ends_with(".mp3"));

    // Episode art healed away from the MP3 enclosure to the feed item's
    // itunes image (the guid-matched fixture item has one).
    let e = EpisodeEntity::find_by_id(ep_id)
        .one(&dbc)
        .await
        .unwrap()
        .unwrap();
    assert_ne!(
        e.art_url.as_deref(),
        Some(stale_mp3),
        "stale MP3 art_url healed from feed"
    );
    assert!(
        e.art_url.as_deref().is_some_and(|u| !u.ends_with(".mp3")),
        "healed art_url is the item image, got {:?}",
        e.art_url
    );
    assert!(e.art_file_path.is_none(), "stale cached path cleared");

    handle.stop().ok();
    let _ = dbc.close().await;
    root.mark_success();
}

#[tokio::test]
async fn test_manual_poll_empty_podcast_list() {
    let mut root = TestRoot::new("polling_empty_podcast");
    let db_path = root.path().join("halogen.db");
    let dbc = connect_and_migrate(&db_path, true)
        .await
        .expect("create test db");
    create_admin_user(&dbc).await;

    let handle = PollingHandle::new(dbc.clone(), Duration::ZERO, 5);

    handle.poll().await.expect("manual poll with no podcasts");

    let count = EpisodeEntity::find()
        .all(&dbc)
        .await
        .expect("fetch episodes")
        .len();
    assert_eq!(
        count, 0,
        "no episodes should be inserted when no podcasts exist"
    );

    handle.stop().ok();
    let _ = dbc.close().await;
    root.mark_success();
}

#[tokio::test]
async fn test_interval_reset() {
    let mut root = TestRoot::new("polling_interval_reset");
    let db_path = root.path().join("halogen.db");
    let dbc = connect_and_migrate(&db_path, true)
        .await
        .expect("create test db");
    create_admin_user(&dbc).await;

    let handle = PollingHandle::new(dbc.clone(), Duration::from_secs(60), 5);
    handle.reset_interval(Duration::from_secs(120));

    handle.start().expect("start polling");
    assert!(handle.is_running());
    handle.stop().expect("stop polling");

    let _ = dbc.close().await;
    root.mark_success();
}

#[tokio::test]
async fn test_sync_with_mock_feed_multiple_episodes() {
    let mut root = TestRoot::new("polling_sync_multiple");
    let db_path = root.path().join("halogen.db");
    let dbc = connect_and_migrate(&db_path, true)
        .await
        .expect("create test db");
    create_admin_user(&dbc).await;

    let mock_server = MockServer::start().await;

    let podcast_id = 1;
    create_podcast(&dbc, podcast_id, &mock_server.uri()).await;

    let fixture = load_fixture("simplecast_the_daily.xml");
    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture))
        .mount(&mock_server)
        .await;

    rss::sync(&dbc, 5, None).await.expect("sync");

    let episodes = EpisodeEntity::find()
        .all(&dbc)
        .await
        .expect("fetch episodes");
    assert!(
        !episodes.is_empty(),
        "sync should have inserted episodes from RSS feed"
    );

    let podcast_episodes = episodes
        .iter()
        .filter(|ep| ep.podcast_id == podcast_id)
        .collect::<Vec<_>>();
    assert!(
        !podcast_episodes.is_empty(),
        "podcast should have episodes after sync"
    );

    let _ = dbc.close().await;
    root.mark_success();
}

/// `no_sync_before` drops episodes published before the cutoff while walking
/// the feed — only newer items are ingested.
#[tokio::test]
async fn test_sync_respects_no_sync_before() {
    let mut root = TestRoot::new("polling_no_sync_before");
    let db_path = root.path().join("halogen.db");
    let dbc = connect_and_migrate(&db_path, true)
        .await
        .expect("create test db");
    create_admin_user(&dbc).await;

    let mock_server = MockServer::start().await;
    let podcast_id = 1;
    create_podcast(&dbc, podcast_id, &mock_server.uri()).await;

    // Two items either side of 2026-01-01.
    let feed = r#"<?xml version="1.0"?>
<rss version="2.0"><channel><title>T</title><link>http://x</link><description>d</description>
<item><title>Old</title><enclosure url="http://x/old.mp3" type="audio/mpeg" length="1"/><pubDate>Mon, 01 Dec 2025 00:00:00 GMT</pubDate><guid>old</guid></item>
<item><title>New</title><enclosure url="http://x/new.mp3" type="audio/mpeg" length="1"/><pubDate>Sun, 01 Feb 2026 00:00:00 GMT</pubDate><guid>new</guid></item>
</channel></rss>"#;
    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200).set_body_string(feed))
        .mount(&mock_server)
        .await;

    let cutoff = chrono::NaiveDate::from_ymd_opt(2026, 1, 1)
        .unwrap()
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc();
    rss::sync(&dbc, 5, Some(cutoff)).await.expect("sync");

    let episodes = EpisodeEntity::find()
        .all(&dbc)
        .await
        .expect("fetch episodes");
    let titles: Vec<&str> = episodes.iter().map(|e| e.title.as_str()).collect();
    assert_eq!(
        titles,
        vec!["New"],
        "only the post-cutoff episode is ingested"
    );

    let _ = dbc.close().await;
    root.mark_success();
}

#[tokio::test]
async fn test_handle_poll_with_mock_feed() {
    let mut root = TestRoot::new("polling_handle_poll");
    let db_path = root.path().join("halogen.db");
    let dbc = connect_and_migrate(&db_path, true)
        .await
        .expect("create test db");
    create_admin_user(&dbc).await;

    let mock_server = MockServer::start().await;

    let podcast_id = 1;
    create_podcast(&dbc, podcast_id, &mock_server.uri()).await;

    let handle = PollingHandle::new(dbc.clone(), Duration::ZERO, 5);

    let fixture = load_fixture("simplecast_the_daily.xml");
    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture))
        .mount(&mock_server)
        .await;

    handle.poll().await.expect("handle poll");

    let episodes = EpisodeEntity::find()
        .all(&dbc)
        .await
        .expect("fetch episodes");
    assert!(
        !episodes.is_empty(),
        "handle poll should have inserted episodes"
    );

    handle.stop().ok();
    let _ = dbc.close().await;
    root.mark_success();
}

#[tokio::test]
async fn test_sync_with_transistor_feed() {
    let mut root = TestRoot::new("polling_sync_transistor");
    let db_path = root.path().join("halogen.db");
    let dbc = connect_and_migrate(&db_path, true)
        .await
        .expect("create test db");
    create_admin_user(&dbc).await;

    let mock_server = MockServer::start().await;

    let podcast_id = 1;
    create_podcast(&dbc, podcast_id, &mock_server.uri()).await;

    let fixture = load_fixture("transistor_acquired.xml");
    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture))
        .mount(&mock_server)
        .await;

    rss::sync(&dbc, 5, None).await.expect("sync");

    let episodes = EpisodeEntity::find()
        .all(&dbc)
        .await
        .expect("fetch episodes");
    assert!(
        !episodes.is_empty(),
        "sync should have inserted episodes from transistor feed"
    );

    let _ = dbc.close().await;
    root.mark_success();
}

#[tokio::test]
async fn test_sync_with_syntax_feed() {
    let mut root = TestRoot::new("polling_sync_syntax");
    let db_path = root.path().join("halogen.db");
    let dbc = connect_and_migrate(&db_path, true)
        .await
        .expect("create test db");
    create_admin_user(&dbc).await;

    let mock_server = MockServer::start().await;

    let podcast_id = 1;
    create_podcast(&dbc, podcast_id, &mock_server.uri()).await;

    let fixture = load_fixture("syntax_fm.xml");
    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture))
        .mount(&mock_server)
        .await;

    rss::sync(&dbc, 5, None).await.expect("sync");

    let episodes = EpisodeEntity::find()
        .all(&dbc)
        .await
        .expect("fetch episodes");
    assert!(
        !episodes.is_empty(),
        "sync should have inserted episodes from syntax.fm feed"
    );

    let _ = dbc.close().await;
    root.mark_success();
}

/// A podcast polled more recently than its resolved interval is skipped on a
/// scheduled (`respect_poll_interval`) run — the feed is never fetched.
#[tokio::test]
async fn sync_respects_per_podcast_poll_interval() {
    let mut root = TestRoot::new("polling_interval_gating");
    let db_path = root.path().join("halogen.db");
    let dbc = connect_and_migrate(&db_path, true)
        .await
        .expect("create test db");
    create_admin_user(&dbc).await;

    let mock_server = MockServer::start().await;
    let podcast_id = 1;
    create_podcast(&dbc, podcast_id, &mock_server.uri()).await;
    // Mark it as just polled.
    PodcastActiveModel {
        id: Set(podcast_id),
        polled_at: Set(Some(chrono::Utc::now())),
        ..Default::default()
    }
    .update(&dbc)
    .await
    .expect("set polled_at");

    // The feed must NOT be fetched (verified on mock drop via `expect(0)`).
    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<rss/>"))
        .expect(0)
        .mount(&mock_server)
        .await;

    let ctx = rss::SyncContext {
        max_poll_concurrent: 5,
        fallback_poll_interval: Duration::from_secs(3600),
        media_root: std::path::PathBuf::from("."),
        respect_poll_interval: true,
        ..Default::default()
    };
    rss::sync_with_context(&dbc, &ctx).await.expect("sync");

    let episodes = EpisodeEntity::find()
        .all(&dbc)
        .await
        .expect("fetch episodes");
    assert!(
        episodes.is_empty(),
        "recently-polled podcast should be skipped (no fetch, no ingest)"
    );

    let _ = dbc.close().await;
    root.mark_success();
}

/// A feed behind a 301 records the full hop chain `feed_url,end_url`.
#[tokio::test]
async fn test_poll_records_redirect_chain() {
    let mut root = TestRoot::new("polling_redirect_chain");
    let db_path = root.path().join("halogen.db");
    let dbc = connect_and_migrate(&db_path, true)
        .await
        .expect("create test db");
    create_admin_user(&dbc).await;

    let mock_server = MockServer::start().await;
    let feed_url = format!("{}/feed", mock_server.uri());
    let real_url = format!("{}/real", mock_server.uri());

    let podcast_id = 1;
    create_podcast(&dbc, podcast_id, &feed_url).await;

    // /feed 301 -> /real, /real 200 + feed body.
    Mock::given(method("GET"))
        .and(path("/feed"))
        .respond_with(ResponseTemplate::new(301).insert_header("Location", real_url.as_str()))
        .mount(&mock_server)
        .await;
    let fixture = load_fixture("simplecast_the_daily.xml");
    Mock::given(method("GET"))
        .and(path("/real"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture))
        .mount(&mock_server)
        .await;

    let handle = PollingHandle::new(dbc.clone(), Duration::ZERO, 5);
    handle.poll().await.expect("manual poll");

    let pod = PodcastEntity::find_by_id(podcast_id)
        .one(&dbc)
        .await
        .expect("fetch podcast")
        .expect("podcast exists");
    assert_eq!(
        pod.feed_url_redirects,
        Some(format!("{feed_url},{real_url}")),
        "redirect hop chain should be recorded as CSV"
    );
    // The followed feed was actually parsed + ingested.
    let episodes = EpisodeEntity::find().all(&dbc).await.expect("episodes");
    assert!(!episodes.is_empty(), "followed feed should ingest episodes");

    handle.stop().ok();
    let _ = dbc.close().await;
    root.mark_success();
}

/// A direct feed (no redirect) records exactly `feed_url`, so the UI's
/// `feed_url_redirects != feed_url` check reads as "no redirect".
#[tokio::test]
async fn test_poll_direct_feed_records_feed_url_only() {
    let mut root = TestRoot::new("polling_direct_feed");
    let db_path = root.path().join("halogen.db");
    let dbc = connect_and_migrate(&db_path, true)
        .await
        .expect("create test db");
    create_admin_user(&dbc).await;

    let mock_server = MockServer::start().await;
    let podcast_id = 1;
    create_podcast(&dbc, podcast_id, &mock_server.uri()).await;

    let fixture = load_fixture("simplecast_the_daily.xml");
    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture))
        .mount(&mock_server)
        .await;

    let handle = PollingHandle::new(dbc.clone(), Duration::ZERO, 5);
    handle.poll().await.expect("manual poll");

    let pod = PodcastEntity::find_by_id(podcast_id)
        .one(&dbc)
        .await
        .expect("fetch podcast")
        .expect("podcast exists");
    assert_eq!(
        pod.feed_url_redirects.as_deref(),
        Some(pod.feed_url.as_str()),
        "direct feed records exactly feed_url"
    );

    handle.stop().ok();
    let _ = dbc.close().await;
    root.mark_success();
}

/// The hop chain is written even when the final response is `304 Not Modified`
/// (a CDN can redirect, then 304). `304` sits in the 3xx range but carries no
/// `Location`, so the manual follower returns it as the final response.
#[tokio::test]
async fn test_poll_records_redirect_chain_on_not_modified() {
    let mut root = TestRoot::new("polling_redirect_304");
    let db_path = root.path().join("halogen.db");
    let dbc = connect_and_migrate(&db_path, true)
        .await
        .expect("create test db");
    create_admin_user(&dbc).await;

    let mock_server = MockServer::start().await;
    let feed_url = format!("{}/feed", mock_server.uri());
    let real_url = format!("{}/real", mock_server.uri());
    let podcast_id = 1;
    create_podcast(&dbc, podcast_id, &feed_url).await;

    // /feed 301 -> /real, /real 304 Not Modified.
    Mock::given(method("GET"))
        .and(path("/feed"))
        .respond_with(ResponseTemplate::new(301).insert_header("Location", real_url.as_str()))
        .mount(&mock_server)
        .await;
    Mock::given(method("GET"))
        .and(path("/real"))
        .respond_with(ResponseTemplate::new(304))
        .mount(&mock_server)
        .await;

    let handle = PollingHandle::new(dbc.clone(), Duration::ZERO, 5);
    handle.poll().await.expect("manual poll");

    let pod = PodcastEntity::find_by_id(podcast_id)
        .one(&dbc)
        .await
        .expect("fetch podcast")
        .expect("podcast exists");
    assert_eq!(
        pod.feed_url_redirects,
        Some(format!("{feed_url},{real_url}")),
        "chain recorded even on the 304 not-modified branch"
    );
    assert!(pod.polled_at.is_some(), "polled_at set on 304");
    let episodes = EpisodeEntity::find().all(&dbc).await.expect("episodes");
    assert!(episodes.is_empty(), "304 ingests nothing");

    handle.stop().ok();
    let _ = dbc.close().await;
    root.mark_success();
}

mod jobs_tests {
    use crate::jobs::*;
    use halogen_fixture::test_support::TestRoot;
    use halogen_migrations::connect_and_migrate;
    use halogen_wire::{PodcastPollOutcome, PodcastPollResultData, PollJobStatus, PollJobTrigger};

    fn result(podcast_id: i32, new: usize) -> PodcastPollResultData {
        PodcastPollResultData {
            podcast_id,
            title: format!("Podcast {podcast_id}"),
            outcome: PodcastPollOutcome::Polled,
            new_episodes: new,
            updated_episodes: 0,
            errors: 0,
        }
    }

    async fn tracker(name: &str) -> (TestRoot, sea_orm::DatabaseConnection, JobTracker) {
        let root = TestRoot::new(name);
        let db_path = root.path().join("halogen.db");
        let dbc = connect_and_migrate(&db_path, true)
            .await
            .expect("create test db");
        let t = JobTracker::new(dbc.clone());
        (root, dbc, t)
    }

    #[tokio::test]
    async fn create_persists_and_totals_fold_in() {
        let (mut root, dbc, t) = tracker("jobs_create_totals").await;
        let a = t.create(PollJobTrigger::Manual, None).await.expect("job a");
        let b = t
            .create(PollJobTrigger::Scheduled, Some(7))
            .await
            .expect("job b");
        assert!(b > a, "ids ascend");

        t.record_podcast(a, result(1, 3)).await;
        t.record_podcast(a, result(2, 2)).await;
        let job = t.get(a).await.expect("job a readable");
        assert_eq!(job.total_new, 5);
        assert_eq!(job.podcasts.len(), 2);
        assert_eq!(job.status, PollJobStatus::Running);
        assert_eq!(job.trigger, PollJobTrigger::Manual);

        let job_b = t.get(b).await.expect("job b readable");
        assert_eq!(job_b.podcast_id, Some(7));
        assert_eq!(job_b.trigger, PollJobTrigger::Scheduled);

        let _ = dbc.close().await;
        root.mark_success();
    }

    #[tokio::test]
    async fn finish_stamps_status_and_survives_a_new_tracker() {
        let (mut root, dbc, t) = tracker("jobs_finish_durable").await;
        let id = t.create(PollJobTrigger::Manual, None).await.expect("job");
        t.finish(id, PollJobStatus::Completed).await;
        let job = t.get(id).await.expect("job readable");
        assert_eq!(job.status, PollJobStatus::Completed);
        assert!(job.completed_at.is_some());

        // The whole point of the DB backing: a fresh tracker (≈ a restarted
        // server) still sees the finished job.
        let fresh = JobTracker::new(dbc.clone());
        assert_eq!(
            fresh.get(id).await.expect("durable job").status,
            PollJobStatus::Completed
        );

        let _ = dbc.close().await;
        root.mark_success();
    }

    #[tokio::test]
    async fn finish_prunes_history_past_max_jobs() {
        let (mut root, dbc, t) = tracker("jobs_prune").await;
        let first = t
            .create(PollJobTrigger::Manual, None)
            .await
            .expect("first job");
        t.record_podcast(first, result(1, 1)).await;
        let mut last = first;
        for _ in 0..MAX_JOBS + 2 {
            last = t.create(PollJobTrigger::Manual, None).await.expect("job");
        }
        // Pruning runs on finish; the oldest jobs (beyond the newest MAX_JOBS)
        // are dropped, their outcome rows cascading with them.
        t.finish(last, PollJobStatus::Completed).await;
        assert!(t.get(first).await.is_none(), "oldest job pruned");
        assert!(t.get(last).await.is_some(), "newest job retained");

        let _ = dbc.close().await;
        root.mark_success();
    }

    #[tokio::test]
    async fn record_on_pruned_job_is_a_noop() {
        let (mut root, dbc, t) = tracker("jobs_record_pruned").await;
        let first = t.create(PollJobTrigger::Manual, None).await.expect("job");
        let mut last = first;
        for _ in 0..MAX_JOBS + 2 {
            last = t.create(PollJobTrigger::Manual, None).await.expect("job");
        }
        t.finish(last, PollJobStatus::Completed).await;
        assert!(t.get(first).await.is_none(), "first pruned");
        // Recording against the pruned id must not panic or resurrect it (the
        // orphaned outcome insert is best-effort and ignored on read).
        t.record_podcast(first, result(1, 1)).await;
        assert!(t.get(first).await.is_none());

        let _ = dbc.close().await;
        root.mark_success();
    }
}

#[tokio::test]
async fn poll_backfills_description_despite_cached_validators_and_preserves_edits() {
    use sea_orm::IntoActiveModel;
    let mut root = TestRoot::new("poll_description_backfill");
    let dbc = connect_and_migrate(&root.path().join("halogen.db"), true)
        .await
        .unwrap();
    create_admin_user(&dbc).await;
    let server = MockServer::start().await;
    create_podcast(&dbc, 1, &server.uri()).await;
    let mut podcast = PodcastEntity::find_by_id(1)
        .one(&dbc)
        .await
        .unwrap()
        .unwrap()
        .into_active_model();
    podcast.description = Set(String::new());
    podcast.etag = Set(Some("cached".into()));
    podcast.last_modified = Set(Some("Tue, 08 Sep 2026 00:00:00 GMT".into()));
    podcast.update(&dbc).await.unwrap();
    Mock::given(method("GET"))
        .respond_with(|request: &wiremock::Request| {
            if request.headers.contains_key("if-none-match") {
                ResponseTemplate::new(304)
            } else {
                ResponseTemplate::new(200).insert_header("ETag", "cached").set_body_string(
                    "<rss version=\"2.0\"><channel><title>Show</title><description><![CDATA[<p>Feed description</p>]]></description></channel></rss>")
            }
        }).mount(&server).await;
    let handle = PollingHandle::new(dbc.clone(), Duration::ZERO, 1);
    handle.poll().await.unwrap();
    let podcast = PodcastEntity::find_by_id(1)
        .one(&dbc)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(podcast.description, "<p>Feed description</p>");
    let requests = server.received_requests().await.unwrap();
    assert!(!requests[0].headers.contains_key("if-none-match"));
    assert!(!requests[0].headers.contains_key("if-modified-since"));
    handle.poll().await.unwrap();
    let requests = server.received_requests().await.unwrap();
    assert!(requests[1].headers.contains_key("if-none-match"));
    assert_eq!(
        PodcastEntity::find_by_id(1)
            .one(&dbc)
            .await
            .unwrap()
            .unwrap()
            .description,
        podcast.description
    );

    server.reset().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            "<rss version=\"2.0\"><channel><title>Show</title><description>Changed feed description</description></channel></rss>"))
        .mount(&server).await;
    let mut edited = podcast.into_active_model();
    edited.description = Set("User description".into());
    edited.update(&dbc).await.unwrap();
    handle.poll().await.unwrap();
    assert_eq!(
        PodcastEntity::find_by_id(1)
            .one(&dbc)
            .await
            .unwrap()
            .unwrap()
            .description,
        "User description"
    );
    dbc.close().await.unwrap();
    root.mark_success();
}

#[tokio::test]
async fn description_edited_during_feed_fetch_is_not_overwritten() {
    use sea_orm::IntoActiveModel;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut root = TestRoot::new("poll_description_concurrent_edit");
    let dbc = connect_and_migrate(&root.path().join("halogen.db"), true)
        .await
        .unwrap();
    create_admin_user(&dbc).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    create_podcast(
        &dbc,
        1,
        &format!("http://{}", listener.local_addr().unwrap()),
    )
    .await;
    let mut podcast = PodcastEntity::find_by_id(1)
        .one(&dbc)
        .await
        .unwrap()
        .unwrap()
        .into_active_model();
    podcast.description = Set(String::new());
    podcast.update(&dbc).await.unwrap();
    let (arrived_tx, arrived_rx) = tokio::sync::oneshot::channel();
    let (respond_tx, respond_rx) = tokio::sync::oneshot::channel();
    let feed = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        assert!(socket.read(&mut [0u8; 4096]).await.unwrap() > 0);
        arrived_tx.send(()).unwrap();
        respond_rx.await.unwrap();
        let body = "<rss version=\"2.0\"><channel><title>Show</title><description>Feed description</description></channel></rss>";
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        socket.write_all(response.as_bytes()).await.unwrap();
    });
    let handle = PollingHandle::new(dbc.clone(), Duration::ZERO, 1);
    let poll = tokio::spawn(async move { handle.poll().await });
    tokio::time::timeout(Duration::from_secs(5), arrived_rx)
        .await
        .unwrap()
        .unwrap();
    PodcastActiveModel {
        id: Set(1),
        description: Set("Edited while fetching".into()),
        ..Default::default()
    }
    .update(&dbc)
    .await
    .unwrap();
    respond_tx.send(()).unwrap();
    poll.await.unwrap().unwrap();
    feed.await.unwrap();
    assert_eq!(
        PodcastEntity::find_by_id(1)
            .one(&dbc)
            .await
            .unwrap()
            .unwrap()
            .description,
        "Edited while fetching"
    );
    dbc.close().await.unwrap();
    root.mark_success();
}
