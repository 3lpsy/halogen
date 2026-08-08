use super::*;
use halogen_migrate::connect_and_migrate;
use halogen_orm::episode::{ActiveModel as EpisodeAM, Model as EpisodeModel};
use halogen_orm::podcast::ActiveModel as PodcastAM;
use sea_orm::ActiveModelTrait;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

use halogen_fixture::test_support::TestRoot;

/// Spin up a fresh migrated SQLite db backed by a `TestRoot` temp dir.
async fn setup_db(suite: &str) -> (TestRoot, DatabaseConnection) {
    let root = TestRoot::new(suite);
    let db_path = root.path().join("halogen.db");
    let dbc = connect_and_migrate(&db_path, true)
        .await
        .expect("create test db");
    // Foreign keys are enforced; the test podcasts use owner_id = 1, so seed a
    // user with that id to satisfy the `podcast.owner_id → user` FK.
    halogen_orm::user::ActiveModel {
        id: Set(1),
        username: Set("owner".to_string()),
        password_hash: Set("x".to_string()),
        is_admin: Set(false),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
    }
    .insert(&dbc)
    .await
    .expect("seed owner user");
    (root, dbc)
}

/// Insert a podcast + a single episode with the given content_url and status,
/// returning the inserted episode id. Ids are negative to avoid collisions.
async fn seed_episode(dbc: &DatabaseConnection, content_url: &str, status: DownloadStatus) -> i32 {
    // Each call gets a unique negative id so multiple seeds can coexist.
    static NEXT_ID: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1000);
    let n = NEXT_ID.fetch_sub(2, std::sync::atomic::Ordering::SeqCst);
    let podcast_id = n;
    let episode_id = n - 1;

    let podcast = PodcastAM {
        id: Set(podcast_id),
        title: Set("Test Podcast".to_string()),
        description: Set("Podcast description".to_string()),
        feed_url: Set(format!("https://example.com/feed{podcast_id}.xml")),
        art_url: Set(None),
        art_file_path: Set(None),
        author: Set(None),
        etag: Set(None),
        last_modified: Set(None),
        polled_at: Set(None),
        podcast_config_id: Set(None),
        owner_id: Set(1),
        feed_url_redirects: Set(None),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
    };
    podcast.insert(dbc).await.expect("insert podcast");

    let episode = EpisodeAM {
        id: Set(episode_id),
        podcast_id: Set(podcast_id),
        title: Set("Test Episode".to_string()),
        description: Set("Episode description".to_string()),
        content_url: Set(content_url.to_string()),
        guid: Set(None),
        art_url: Set(None),
        published_at: Set(Some(Utc::now())),
        downloaded_at: Set(None),
        content_file_path: Set(None),
        download_size: Set(None),
        art_file_path: Set(None),
        download_status: Set(status),
        download_started_at: Set(None),
        download_attempts: Set(0),
        duration_secs: Set(None),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
    };
    episode.insert(dbc).await.expect("insert episode");

    episode_id
}

/// Build `DownloadOptions` for tests: a fresh tracker and NO retry/backoff so
/// tests never sleep on the transient path.
fn test_opts(media_root: &Path) -> DownloadOptions {
    DownloadOptions {
        media_root: media_root.to_path_buf(),
        use_mock_download: false,
        tracker: Arc::new(DownloadTracker::new()),
        retry: RetryPolicy::none(),
    }
}

/// Fetch an episode Model back from the db by id.
async fn fetch_episode(dbc: &DatabaseConnection, id: i32) -> EpisodeModel {
    EpisodeEntity::find()
        .filter(EpisodeColumn::Id.eq(id))
        .one(dbc)
        .await
        .expect("query episode")
        .expect("episode exists")
}

/// Build an episode Model with a chosen content_url so we can exercise
/// `extract_extension` without standing up a server.
async fn model_with_url(dbc: &DatabaseConnection, url: &str) -> EpisodeModel {
    let id = seed_episode(dbc, url, DownloadStatus::NotDownloaded).await;
    fetch_episode(dbc, id).await
}

#[tokio::test]
async fn extract_extension_handles_various_urls() {
    let (mut root, dbc) = setup_db("test_download").await;

    // Plain extensions are kept and lowercased with a leading dot.
    let m = model_with_url(&dbc, "https://example.com/audio/ep.mp3").await;
    assert_eq!(extract_extension(&m), ".mp3");

    let m = model_with_url(&dbc, "https://example.com/audio/ep.m4a").await;
    assert_eq!(extract_extension(&m), ".m4a");

    // Query strings are stripped before extension detection.
    let m = model_with_url(&dbc, "https://example.com/ep.mp3?foo=bar&x=1").await;
    assert_eq!(extract_extension(&m), ".mp3");

    // Fragments are stripped too.
    let m = model_with_url(&dbc, "https://example.com/ep.aac#frag").await;
    assert_eq!(extract_extension(&m), ".aac");

    // No extension falls back to .wav.
    let m = model_with_url(&dbc, "https://example.com/audio/episode").await;
    assert_eq!(extract_extension(&m), ".wav");

    // Uppercase extensions are lowercased.
    let m = model_with_url(&dbc, "https://example.com/ep.MP3").await;
    assert_eq!(extract_extension(&m), ".mp3");

    // Overly long (>= 10 chars) fake extensions fall back to .wav.
    let m = model_with_url(&dbc, "https://example.com/file.superlongext").await;
    assert_eq!(extract_extension(&m), ".wav");

    // Non-alphanumeric extensions fall back to .wav.
    let m = model_with_url(&dbc, "https://example.com/file.mp3!").await;
    assert_eq!(extract_extension(&m), ".wav");

    root.mark_success();
}

#[tokio::test]
async fn begin_attempt_claims_atomically() {
    let (mut root, dbc) = setup_db("test_download").await;
    let id = seed_episode(
        &dbc,
        "https://example.com/ep.mp3",
        DownloadStatus::DownloadError,
    )
    .await;
    let episode = fetch_episode(&dbc, id).await;

    // First claim wins: the compare-and-swap flips the row to Downloading and
    // bumps the attempt counter exactly once.
    assert!(
        begin_attempt(&dbc, &episode).await.expect("claim ok"),
        "first claim should win"
    );
    let claimed = fetch_episode(&dbc, id).await;
    assert_eq!(claimed.download_status, DownloadStatus::Downloading);
    assert_eq!(claimed.download_attempts, 1);

    // A concurrent claim (same stale model) loses — the row is already
    // Downloading — so it must NOT bump the counter again or proceed to fetch.
    assert!(
        !begin_attempt(&dbc, &episode).await.expect("claim ok"),
        "second claim should lose the race"
    );
    assert_eq!(fetch_episode(&dbc, id).await.download_attempts, 1);

    root.mark_success();
}

#[tokio::test]
async fn download_episode_skips_when_already_downloaded() {
    let (mut root, dbc) = setup_db("test_download").await;
    let media_root = root.path().join("media");

    let id = seed_episode(
        &dbc,
        "https://example.com/ep.mp3",
        DownloadStatus::Downloaded,
    )
    .await;

    // Already downloaded: returns Ok and leaves the status untouched.
    download_episode(&dbc, id, &test_opts(&media_root))
        .await
        .expect("download_episode ok");

    let ep = fetch_episode(&dbc, id).await;
    assert_eq!(ep.download_status, DownloadStatus::Downloaded);

    root.mark_success();
}

#[tokio::test]
async fn download_episode_skips_when_downloading() {
    let (mut root, dbc) = setup_db("test_download").await;
    let media_root = root.path().join("media");

    let id = seed_episode(
        &dbc,
        "https://example.com/ep.mp3",
        DownloadStatus::Downloading,
    )
    .await;

    // In-flight download: returns Ok and leaves the status untouched.
    download_episode(&dbc, id, &test_opts(&media_root))
        .await
        .expect("download_episode ok");

    let ep = fetch_episode(&dbc, id).await;
    assert_eq!(ep.download_status, DownloadStatus::Downloading);

    root.mark_success();
}

#[tokio::test]
async fn download_episode_happy_path_remote() {
    let (mut root, dbc) = setup_db("test_download").await;
    let media_root = root.path().join("media");

    // Serve some bytes for any GET.
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"AUDIO".to_vec()))
        .mount(&server)
        .await;

    let url = format!("{}/x.mp3", server.uri());
    let id = seed_episode(&dbc, &url, DownloadStatus::NotDownloaded).await;

    download_episode(&dbc, id, &test_opts(&media_root))
        .await
        .expect("download_episode ok");

    let ep = fetch_episode(&dbc, id).await;
    assert_eq!(ep.download_status, DownloadStatus::Downloaded);

    // The final path is recorded and the file actually exists on disk.
    let path = ep.content_file_path.expect("content_file_path set");
    assert!(Path::new(&path).exists(), "downloaded file should exist");
    let bytes = std::fs::read(&path).expect("read downloaded file");
    assert_eq!(bytes, b"AUDIO");

    root.mark_success();
}

#[tokio::test]
async fn download_episode_missing_id_errors() {
    let (mut root, dbc) = setup_db("test_download").await;
    let media_root = root.path().join("media");

    // No episode with this id exists -> Err.
    let result = download_episode(&dbc, 999_999, &test_opts(&media_root)).await;
    assert!(result.is_err(), "missing episode should error");

    root.mark_success();
}

#[tokio::test]
async fn download_remote_http_error_leaves_no_file() {
    let (mut root, dbc) = setup_db("test_download").await;
    let media_root = root.path().join("media");

    // Server replies 500 for any GET.
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let url = format!("{}/x.mp3", server.uri());
    let id = seed_episode(&dbc, &url, DownloadStatus::NotDownloaded).await;
    let episode = fetch_episode(&dbc, id).await;

    let client = download_client();
    let result = download_remote(&client, &episode, &media_root, &DownloadTracker::new()).await;
    assert!(
        matches!(result, Err(DownloadFailure::Transient(_))),
        "500 should classify as Transient (retryable)"
    );

    // The would-be final file must not exist (extension is .mp3 here).
    let dest = media_root.join(format!("{id}.mp3"));
    assert!(!dest.exists(), "no final file on http error");

    root.mark_success();
}

#[tokio::test]
async fn download_remote_404_errors() {
    let (mut root, dbc) = setup_db("test_download").await;
    let media_root = root.path().join("media");

    // Server replies 404 for any GET.
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let url = format!("{}/missing.mp3", server.uri());
    let id = seed_episode(&dbc, &url, DownloadStatus::NotDownloaded).await;
    let episode = fetch_episode(&dbc, id).await;

    let client = download_client();
    let result = download_remote(&client, &episode, &media_root, &DownloadTracker::new()).await;
    assert!(
        matches!(result, Err(DownloadFailure::RemoteNotFound)),
        "404 should classify as RemoteNotFound (terminal)"
    );

    root.mark_success();
}

#[tokio::test]
async fn set_failed_sets_terminal_status() {
    let (mut root, dbc) = setup_db("test_download").await;

    let id = seed_episode(
        &dbc,
        "https://example.com/ep.mp3",
        DownloadStatus::Downloading,
    )
    .await;

    set_failed(&dbc, id, DownloadStatus::DownloadError)
        .await
        .expect("set_failed ok");
    assert_eq!(
        fetch_episode(&dbc, id).await.download_status,
        DownloadStatus::DownloadError
    );

    // Works for the other terminal states too.
    set_failed(&dbc, id, DownloadStatus::DownloadUnauthorized)
        .await
        .expect("set_failed ok");
    assert_eq!(
        fetch_episode(&dbc, id).await.download_status,
        DownloadStatus::DownloadUnauthorized
    );

    root.mark_success();
}

#[tokio::test]
async fn download_episode_403_sets_unauthorized() {
    let (mut root, dbc) = setup_db("test_download").await;
    let media_root = root.path().join("media");

    // Server replies 403 for any GET — simulates the buzzsprout case.
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(403))
        .mount(&server)
        .await;

    let url = format!("{}/x.mp3", server.uri());
    let id = seed_episode(&dbc, &url, DownloadStatus::NotDownloaded).await;

    let result = download_episode(&dbc, id, &test_opts(&media_root)).await;
    assert!(result.is_err(), "download should fail on 403");

    let ep = fetch_episode(&dbc, id).await;
    assert_eq!(
        ep.download_status,
        DownloadStatus::DownloadUnauthorized,
        "403 is terminal → DownloadUnauthorized"
    );
    assert_eq!(ep.download_attempts, 1, "one attempt counted");
    assert!(ep.download_started_at.is_some(), "start stamped");

    root.mark_success();
}

// A manual re-download from an error/terminal state re-attempts (the skip-guard
// only short-circuits Downloading/Downloaded) and reclassifies the outcome.
#[tokio::test]
async fn download_episode_retries_from_error_state_and_reclassifies() {
    let (mut root, dbc) = setup_db("test_download").await;
    let media_root = root.path().join("media");

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(403))
        .mount(&server)
        .await;

    let url = format!("{}/x.mp3", server.uri());
    let id = seed_episode(&dbc, &url, DownloadStatus::DownloadError).await;

    let result = download_episode(&dbc, id, &test_opts(&media_root)).await;
    assert!(result.is_err(), "403 persists → still fails");

    let ep = fetch_episode(&dbc, id).await;
    assert_eq!(
        ep.download_status,
        DownloadStatus::DownloadUnauthorized,
        "re-attempt reclassifies 403 → DownloadUnauthorized"
    );
    assert_eq!(ep.download_attempts, 1, "the re-attempt was counted");

    root.mark_success();
}

// 500 then 200: the in-call retry loop recovers within the attempt budget and
// the episode ends Downloaded.
#[tokio::test]
async fn download_episode_retries_transient_then_succeeds() {
    let (mut root, dbc) = setup_db("test_download").await;
    let media_root = root.path().join("media");

    let server = MockServer::start().await;
    // First call: 500 (transient) — higher priority + capped to one match so it
    // wins the first request, then falls through. Subsequent calls: 200 + body.
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(500))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"AUDIO".to_vec()))
        .mount(&server)
        .await;

    let url = format!("{}/x.mp3", server.uri());
    let id = seed_episode(&dbc, &url, DownloadStatus::NotDownloaded).await;

    // Multi-attempt with zero backoff so the retry runs instantly.
    let opts = DownloadOptions {
        media_root: media_root.clone(),
        use_mock_download: false,
        tracker: Arc::new(DownloadTracker::new()),
        retry: RetryPolicy {
            attempts: 4,
            backoff_base: Duration::ZERO,
        },
    };

    download_episode(&dbc, id, &opts)
        .await
        .expect("should succeed after retry");

    let ep = fetch_episode(&dbc, id).await;
    assert_eq!(ep.download_status, DownloadStatus::Downloaded);
    // The whole call counts as one attempt, regardless of in-call retries.
    assert_eq!(
        ep.download_attempts, 1,
        "in-call retries don't bump attempts"
    );

    root.mark_success();
}

// The boot reconcile flips every orphaned `Downloading` row to `DownloadError`.
#[tokio::test]
async fn reclaim_orphaned_resets_all_downloading() {
    let (mut root, dbc) = setup_db("test_download").await;

    let downloading = seed_episode(
        &dbc,
        "https://example.com/a.mp3",
        DownloadStatus::Downloading,
    )
    .await;
    let downloaded = seed_episode(
        &dbc,
        "https://example.com/b.mp3",
        DownloadStatus::Downloaded,
    )
    .await;

    let reset = reclaim_orphaned_downloads(&dbc).await.expect("reclaim ok");
    assert_eq!(reset, 1, "only the Downloading row is reset");
    assert_eq!(
        fetch_episode(&dbc, downloading).await.download_status,
        DownloadStatus::DownloadError
    );
    assert_eq!(
        fetch_episode(&dbc, downloaded).await.download_status,
        DownloadStatus::Downloaded,
        "completed downloads untouched"
    );

    root.mark_success();
}

// The watchdog (Some cutoff) resets only rows started before the cutoff (or
// with a NULL start); a freshly-started row is left alone.
#[tokio::test]
async fn watchdog_resets_only_stale_downloading() {
    let (mut root, dbc) = setup_db("test_download").await;

    let stale = seed_episode(
        &dbc,
        "https://example.com/s.mp3",
        DownloadStatus::Downloading,
    )
    .await;
    let fresh = seed_episode(
        &dbc,
        "https://example.com/f.mp3",
        DownloadStatus::Downloading,
    )
    .await;

    // stale started 10h ago; fresh started just now.
    let stale_update = EpisodeActiveModel {
        id: Set(stale),
        download_started_at: Set(Some(Utc::now() - chrono::Duration::hours(10))),
        ..Default::default()
    };
    EpisodeEntity::update(stale_update)
        .exec(&dbc)
        .await
        .expect("stamp stale");
    let fresh_update = EpisodeActiveModel {
        id: Set(fresh),
        download_started_at: Set(Some(Utc::now())),
        ..Default::default()
    };
    EpisodeEntity::update(fresh_update)
        .exec(&dbc)
        .await
        .expect("stamp fresh");

    let cutoff = Utc::now() - chrono::Duration::hours(6);
    let reset = reset_stuck_downloads(&dbc, Some(cutoff))
        .await
        .expect("watchdog ok");
    assert_eq!(reset, 1, "only the stale row is reset");
    assert_eq!(
        fetch_episode(&dbc, stale).await.download_status,
        DownloadStatus::DownloadError
    );
    assert_eq!(
        fetch_episode(&dbc, fresh).await.download_status,
        DownloadStatus::Downloading,
        "fresh in-flight download left running"
    );

    root.mark_success();
}

// The broken cap flips `DownloadError` rows at/over the attempt budget to
// `DownloadBroken`, leaving rows under the budget retryable.
#[tokio::test]
async fn mark_broken_caps_exhausted_errors() {
    let (mut root, dbc) = setup_db("test_download").await;

    let exhausted = seed_episode(
        &dbc,
        "https://example.com/x.mp3",
        DownloadStatus::DownloadError,
    )
    .await;
    let retryable = seed_episode(
        &dbc,
        "https://example.com/y.mp3",
        DownloadStatus::DownloadError,
    )
    .await;

    for (id, attempts) in [(exhausted, 10), (retryable, 3)] {
        let upd = EpisodeActiveModel {
            id: Set(id),
            download_attempts: Set(attempts),
            ..Default::default()
        };
        EpisodeEntity::update(upd)
            .exec(&dbc)
            .await
            .expect("set attempts");
    }

    let marked = mark_broken(&dbc, 10).await.expect("mark_broken ok");
    assert_eq!(marked, 1, "only the exhausted row is capped");
    assert_eq!(
        fetch_episode(&dbc, exhausted).await.download_status,
        DownloadStatus::DownloadBroken
    );
    assert_eq!(
        fetch_episode(&dbc, retryable).await.download_status,
        DownloadStatus::DownloadError,
        "under-budget row stays retryable"
    );

    root.mark_success();
}

#[tokio::test]
async fn download_mock_copies_fixture_clip() {
    let (mut root, dbc) = setup_db("test_download").await;
    let media_root = root.path().join("media");

    let id = seed_episode(
        &dbc,
        "https://example.com/ep.mp3",
        DownloadStatus::NotDownloaded,
    )
    .await;
    let episode = fetch_episode(&dbc, id).await;

    // Copies the bundled nasa-test-clip.mp3 to "{id}_mock.wav".
    let path = download_mock(&episode, &media_root)
        .await
        .expect("download_mock ok");

    let dest = media_root.join(format!("{id}_mock.wav"));
    assert_eq!(path, dest.to_string_lossy());
    assert!(dest.exists(), "mock clip should be copied to dest");
    let meta = std::fs::metadata(&dest).expect("stat mock copy");
    assert!(meta.len() > 0, "copied mock clip should be non-empty");

    root.mark_success();
}

// A successful download records `download_size` + `downloaded_at`; removing it
// clears both (and the status/path).
#[tokio::test]
async fn download_records_size_and_remove_clears_it() {
    let (mut root, dbc) = setup_db("test_download").await;
    let media_root = root.path().join("media");

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"AUDIOAUDIO".to_vec()))
        .mount(&server)
        .await;
    let url = format!("{}/x.mp3", server.uri());
    let id = seed_episode(&dbc, &url, DownloadStatus::NotDownloaded).await;

    download_episode(&dbc, id, &test_opts(&media_root))
        .await
        .expect("download ok");
    let ep = fetch_episode(&dbc, id).await;
    assert_eq!(ep.download_status, DownloadStatus::Downloaded);
    assert_eq!(ep.download_size, Some(10), "size = bytes written");
    assert!(ep.downloaded_at.is_some(), "downloaded_at set on download");

    remove_server_download(&dbc, id).await.expect("remove ok");
    let ep = fetch_episode(&dbc, id).await;
    assert_eq!(ep.download_status, DownloadStatus::NotDownloaded);
    assert_eq!(ep.download_size, None, "size cleared on remove");
    assert!(
        ep.downloaded_at.is_none(),
        "downloaded_at cleared on remove"
    );

    root.mark_success();
}

// Retention keeps the newest `keep` downloads (by `downloaded_at`) and purges
// the rest — files and DB state both reset.
#[tokio::test]
async fn enforce_retention_purges_oldest_over_cap() {
    use halogen_orm::podcast::ActiveModel as PodcastAM;

    let (mut root, dbc) = setup_db("test_download").await;
    let podcast_id = -5000;
    PodcastAM {
        id: Set(podcast_id),
        title: Set("Ret".to_string()),
        description: Set("d".to_string()),
        feed_url: Set("https://example.com/ret.xml".to_string()),
        art_url: Set(None),
        art_file_path: Set(None),
        author: Set(None),
        etag: Set(None),
        last_modified: Set(None),
        polled_at: Set(None),
        podcast_config_id: Set(None),
        owner_id: Set(1),
        feed_url_redirects: Set(None),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
    }
    .insert(&dbc)
    .await
    .expect("insert podcast");

    // Three downloaded episodes with increasing downloaded_at + real files.
    let mut ids = Vec::new();
    for i in 0..3i32 {
        let f = root.path().join(format!("r{i}.mp3"));
        std::fs::write(&f, b"x").expect("write file");
        let ep_id = -6000 - i;
        EpisodeAM {
            id: Set(ep_id),
            podcast_id: Set(podcast_id),
            title: Set(format!("E{i}")),
            description: Set("d".to_string()),
            content_url: Set(format!("https://example.com/e{i}.mp3")),
            guid: Set(None),
            art_url: Set(None),
            published_at: Set(Some(Utc::now())),
            downloaded_at: Set(Some(Utc::now() + chrono::Duration::seconds(i as i64))),
            content_file_path: Set(Some(f.to_string_lossy().to_string())),
            download_size: Set(Some(1)),
            art_file_path: Set(None),
            download_status: Set(DownloadStatus::Downloaded),
            download_started_at: Set(None),
            download_attempts: Set(0),
            duration_secs: Set(None),
            created_at: Set(Utc::now()),
            updated_at: Set(Utc::now()),
        }
        .insert(&dbc)
        .await
        .expect("insert episode");
        ids.push(ep_id);
    }

    let purged = enforce_retention(&dbc, podcast_id, 2)
        .await
        .expect("retention");
    assert_eq!(purged, 1, "one over the cap of 2");

    // ids[0] has the smallest downloaded_at → purged; ids[2] kept.
    let oldest = fetch_episode(&dbc, ids[0]).await;
    assert_eq!(oldest.download_status, DownloadStatus::NotDownloaded);
    assert_eq!(oldest.download_size, None);
    let newest = fetch_episode(&dbc, ids[2]).await;
    assert_eq!(newest.download_status, DownloadStatus::Downloaded);

    root.mark_success();
}

mod tracker_tests {
    use crate::tracker::*;

    #[test]
    fn begin_get_finish_lifecycle() {
        let t = DownloadTracker::new();
        assert!(t.get(1).is_none(), "untracked → None");

        let entry = t.begin(1, Some(100));
        entry.add(25);

        let p = t.get(1).expect("tracked");
        assert_eq!(p.episode_id, 1);
        assert_eq!(p.bytes_downloaded, 25);
        assert_eq!(p.total_bytes, Some(100));
        assert_eq!(p.percent, Some(0.25));

        t.finish(1);
        assert!(t.get(1).is_none(), "removed on finish");
    }

    #[test]
    fn begin_resets_counter() {
        let t = DownloadTracker::new();
        let first = t.begin(7, Some(10));
        first.add(9);
        // A retried attempt re-begins: counter back to 0.
        let _second = t.begin(7, Some(10));
        assert_eq!(t.get(7).unwrap().bytes_downloaded, 0);
    }

    #[test]
    fn percent_is_none_without_total() {
        let t = DownloadTracker::new();
        let e = t.begin(2, None);
        e.add(50);
        let p = t.get(2).unwrap();
        assert_eq!(p.total_bytes, None);
        assert_eq!(p.percent, None);
        assert_eq!(p.bytes_downloaded, 50);
    }

    #[test]
    fn active_lists_all_in_flight() {
        let t = DownloadTracker::new();
        t.begin(1, Some(10));
        t.begin(2, None);
        let mut ids: Vec<i32> = t.active().iter().map(|p| p.episode_id).collect();
        ids.sort_unstable();
        assert_eq!(ids, vec![1, 2]);
        t.finish(1);
        assert_eq!(t.active().len(), 1);
    }
}
