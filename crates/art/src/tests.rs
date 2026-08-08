use super::*;
use halogen_fixture::test_support::TestRoot;
use halogen_migrate::connect_and_migrate;
use sea_orm::ActiveValue;
use wiremock::matchers::{method, path as path_matcher};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// A wiremock serving `/img` as a PNG and `/audio` as an MP3 (the bug).
async fn art_server() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path_matcher("/img"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "image/png")
                .set_body_bytes(b"\x89PNG\r\n\x1a\nfake".to_vec()),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path_matcher("/audio"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "audio/mpeg")
                .set_body_bytes(b"ID3fake-audio".to_vec()),
        )
        .mount(&server)
        .await;
    server
}

async fn db(suite: &str) -> (TestRoot, DatabaseConnection, PathBuf) {
    let mut root = TestRoot::new(suite);
    let db_path = root.path().join("halogen.db");
    let dbc = connect_and_migrate(&db_path, true).await.unwrap();
    // Foreign keys are enforced; the seeded podcasts use owner_id = 1, so seed
    // a user with that id to satisfy the `podcast.owner_id → user` FK.
    halogen_orm::user::ActiveModel {
        id: sea_orm::ActiveValue::set(1),
        username: sea_orm::ActiveValue::set("owner".to_string()),
        password_hash: sea_orm::ActiveValue::set("x".to_string()),
        is_admin: sea_orm::ActiveValue::set(false),
        created_at: sea_orm::ActiveValue::set(chrono::Utc::now()),
        updated_at: sea_orm::ActiveValue::set(chrono::Utc::now()),
    }
    .insert(&dbc)
    .await
    .unwrap();
    let media = root.path().join("media");
    root.mark_success();
    (root, dbc, media)
}

async fn seed_podcast(dbc: &DatabaseConnection, art_url: Option<String>) -> i32 {
    let now = chrono::Utc::now();
    let p = podcast::ActiveModel {
        title: ActiveValue::set("P".into()),
        description: ActiveValue::set("d".into()),
        feed_url: ActiveValue::set("https://feed.test/p".into()),
        art_url: ActiveValue::set(art_url),
        owner_id: ActiveValue::set(1),
        created_at: ActiveValue::set(now),
        updated_at: ActiveValue::set(now),
        ..Default::default()
    };
    p.insert(dbc).await.unwrap().id
}

async fn seed_episode(
    dbc: &DatabaseConnection,
    podcast_id: i32,
    art_url: Option<String>,
    published_at: chrono::DateTime<chrono::Utc>,
) -> i32 {
    let now = chrono::Utc::now();
    let e = episode::ActiveModel {
        podcast_id: ActiveValue::set(podcast_id),
        title: ActiveValue::set("E".into()),
        description: ActiveValue::set("d".into()),
        content_url: ActiveValue::set("https://feed.test/e.mp3".into()),
        art_url: ActiveValue::set(art_url),
        published_at: ActiveValue::set(Some(published_at)),
        created_at: ActiveValue::set(now),
        updated_at: ActiveValue::set(now),
        ..Default::default()
    };
    e.insert(dbc).await.unwrap().id
}

// 1) Episode art is audio (not an image) → falls back to the podcast image.
#[tokio::test]
async fn episode_non_image_falls_back_to_podcast() {
    let (_root, dbc, media) = db("art_ep_audio_fallback").await;
    let srv = art_server().await;
    let podcast_id = seed_podcast(&dbc, Some(format!("{}/img", srv.uri()))).await;
    let ep = seed_episode(
        &dbc,
        podcast_id,
        Some(format!("{}/audio", srv.uri())),
        chrono::Utc::now(),
    )
    .await;

    let got = ensure_episode_art(&dbc, ep, &media, true).await.unwrap();
    let path = got.expect("falls back to podcast art");
    assert!(path.display().to_string().contains("podcast_"));
    assert!(path.is_file());
}

// 2) Episode has no art_url → falls back to the podcast image.
#[tokio::test]
async fn episode_no_art_falls_back_to_podcast() {
    let (_root, dbc, media) = db("art_ep_none_fallback").await;
    let srv = art_server().await;
    let podcast_id = seed_podcast(&dbc, Some(format!("{}/img", srv.uri()))).await;
    let ep = seed_episode(&dbc, podcast_id, None, chrono::Utc::now()).await;

    let got = ensure_episode_art(&dbc, ep, &media, true).await.unwrap();
    assert!(
        got.expect("podcast fallback")
            .display()
            .to_string()
            .contains("podcast_")
    );
}

// 3) Podcast art is audio → falls back to the LATEST-published episode's image.
#[tokio::test]
async fn podcast_non_image_falls_back_to_latest_episode() {
    let (_root, dbc, media) = db("art_pod_audio_fallback").await;
    let srv = art_server().await;
    let podcast_id = seed_podcast(&dbc, Some(format!("{}/audio", srv.uri()))).await;
    let now = chrono::Utc::now();
    // Older episode with no art; newest episode with a valid image.
    seed_episode(&dbc, podcast_id, None, now - chrono::Duration::days(2)).await;
    let newest = seed_episode(&dbc, podcast_id, Some(format!("{}/img", srv.uri())), now).await;

    let got = ensure_podcast_art(&dbc, podcast_id, &media, true)
        .await
        .unwrap();
    let path = got.expect("falls back to latest episode art");
    assert_eq!(
        path,
        media.join("art").join(format!("episode_{newest}.png"))
    );
}

// 4) Both bad → no art, no infinite loop (each hop exhausts to None).
#[tokio::test]
async fn both_non_image_returns_none() {
    let (_root, dbc, media) = db("art_both_bad").await;
    let srv = art_server().await;
    let podcast_id = seed_podcast(&dbc, Some(format!("{}/audio", srv.uri()))).await;
    let ep = seed_episode(
        &dbc,
        podcast_id,
        Some(format!("{}/audio", srv.uri())),
        chrono::Utc::now(),
    )
    .await;

    assert!(
        ensure_episode_art(&dbc, ep, &media, true)
            .await
            .unwrap()
            .is_none()
    );
}

// 5) Fallback disabled → bad episode art returns None, never touches the podcast.
#[tokio::test]
async fn fallback_disabled_returns_none() {
    let (_root, dbc, media) = db("art_no_fallback").await;
    let srv = art_server().await;
    let podcast_id = seed_podcast(&dbc, Some(format!("{}/img", srv.uri()))).await;
    let ep = seed_episode(
        &dbc,
        podcast_id,
        Some(format!("{}/audio", srv.uri())),
        chrono::Utc::now(),
    )
    .await;

    assert!(
        ensure_episode_art(&dbc, ep, &media, false)
            .await
            .unwrap()
            .is_none()
    );
}

// 7) Negative cache: a failing URL is fetched ONCE; repeat resolutions
//    within the TTL skip the origin entirely (the log showed the same
//    broken episode crawled 5× in 25s before this existed).
#[tokio::test]
async fn failed_fetch_is_not_retried_within_ttl() {
    let (_root, dbc, media) = db("art_negative_cache").await;
    let srv = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path_matcher("/audio"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "audio/mpeg")
                .set_body_bytes(b"ID3fake-audio".to_vec()),
        )
        .expect(1) // the whole point: exactly one origin hit
        .mount(&srv)
        .await;
    let podcast_id = seed_podcast(&dbc, None).await;
    let ep = seed_episode(
        &dbc,
        podcast_id,
        Some(format!("{}/audio", srv.uri())),
        chrono::Utc::now(),
    )
    .await;

    for _ in 0..4 {
        assert!(
            ensure_episode_art(&dbc, ep, &media, false)
                .await
                .unwrap()
                .is_none()
        );
    }
    srv.verify().await;
}

// 6) Happy path: a real image is cached and the path persisted.
#[tokio::test]
async fn image_is_cached_and_persisted() {
    let (_root, dbc, media) = db("art_image_ok").await;
    let srv = art_server().await;
    let podcast_id = seed_podcast(&dbc, None).await;
    let ep = seed_episode(
        &dbc,
        podcast_id,
        Some(format!("{}/img", srv.uri())),
        chrono::Utc::now(),
    )
    .await;

    let got = ensure_episode_art(&dbc, ep, &media, true).await.unwrap();
    assert_eq!(
        got,
        Some(media.join("art").join(format!("episode_{ep}.png")))
    );
    let reloaded = episode::Entity::find_by_id(ep)
        .one(&dbc)
        .await
        .unwrap()
        .unwrap();
    assert!(reloaded.art_file_path.is_some(), "art_file_path persisted");
}

/// Encode a real solid-colour PNG of the given size at `path`.
fn write_png(path: &Path, w: u32, h: u32) {
    let buf = image::RgbImage::from_pixel(w, h, image::Rgb([10, 20, 30]));
    image::DynamicImage::ImageRgb8(buf)
        .save_with_format(path, image::ImageFormat::Png)
        .unwrap();
}

// 7) Small variant: a large cached PNG original yields a downscaled, WebP-encoded
//    `<stem>.small.webp` sibling clamped to SMALL_MAX_DIM with aspect preserved.
//    (PNG → lossless WebP shrinks the bytes with no quality loss.)
#[tokio::test]
async fn small_variant_is_generated_and_downscaled() {
    let (_root, dbc, media) = db("art_small_ok").await;
    let podcast_id = seed_podcast(&dbc, None).await;
    let ep = seed_episode(&dbc, podcast_id, None, chrono::Utc::now()).await;

    // Pre-place a large real PNG as the already-cached original.
    let art_dir = media.join("art");
    std::fs::create_dir_all(&art_dir).unwrap();
    let original = art_dir.join(format!("episode_{ep}.png"));
    write_png(&original, 800, 600);
    let mut update = episode::ActiveModel {
        id: ActiveValue::set(ep),
        ..Default::default()
    };
    update.art_file_path = ActiveValue::set(Some(original.display().to_string()));
    update.update(&dbc).await.unwrap();

    // PNG origin → WebP small variant.
    let small = art_dir.join(format!("episode_{ep}.small.webp"));
    let got = ensure_episode_art_small(&dbc, ep, &media, true)
        .await
        .unwrap();
    assert_eq!(got, Some(small.clone()));
    assert!(small.is_file(), "small variant written to disk");

    let reader = image::ImageReader::open(&small)
        .unwrap()
        .with_guessed_format()
        .unwrap();
    // The bytes must really be WebP (proves the lossless WebP encode path works).
    assert_eq!(reader.format(), Some(image::ImageFormat::WebP));
    let decoded = reader.decode().unwrap();
    // 800x600 → longest edge clamped to SMALL_MAX_DIM (160), aspect preserved → 160x120.
    assert_eq!((decoded.width(), decoded.height()), (SMALL_MAX_DIM, 120));
}

// 7b) A non-PNG (JPEG) original keeps its own format — lossless WebP would *grow* a
//     photographic JPEG, so we only convert PNGs.
#[tokio::test]
async fn small_variant_preserves_jpeg_format() {
    let mut root = TestRoot::new("art_small_jpeg");
    let dir = root.path().join("art");
    std::fs::create_dir_all(&dir).unwrap();
    let original = dir.join("episode_5.jpg");
    image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
        800,
        600,
        image::Rgb([10, 20, 30]),
    ))
    .save_with_format(&original, image::ImageFormat::Jpeg)
    .unwrap();

    let small = dir.join("episode_5.small.jpg");
    assert_eq!(ensure_small_variant(&original).await, small);
    let reader = image::ImageReader::open(&small)
        .unwrap()
        .with_guessed_format()
        .unwrap();
    assert_eq!(reader.format(), Some(image::ImageFormat::Jpeg));
    let decoded = reader.decode().unwrap();
    assert_eq!((decoded.width(), decoded.height()), (SMALL_MAX_DIM, 120));
    root.mark_success();
}

// 8) Generation never upscales a small original.
#[tokio::test]
async fn small_variant_never_upscales() {
    let mut root = TestRoot::new("art_small_noup");
    let dir = root.path().join("art");
    std::fs::create_dir_all(&dir).unwrap();
    let original = dir.join("episode_3.png");
    write_png(&original, 100, 80);

    let small = dir.join("episode_3.small.webp");
    assert_eq!(ensure_small_variant(&original).await, small);
    let decoded = image::ImageReader::open(&small)
        .unwrap()
        .with_guessed_format()
        .unwrap()
        .decode()
        .unwrap();
    assert_eq!((decoded.width(), decoded.height()), (100, 80));
    root.mark_success();
}

// 9) An undecodable original degrades gracefully: serve the original, write nothing.
#[tokio::test]
async fn small_variant_falls_back_on_undecodable_original() {
    let mut root = TestRoot::new("art_small_bad");
    let dir = root.path().join("art");
    std::fs::create_dir_all(&dir).unwrap();
    let original = dir.join("podcast_9.png");
    std::fs::write(&original, b"\x89PNG\r\n\x1a\nnot-a-real-image").unwrap();

    assert_eq!(ensure_small_variant(&original).await, original);
    assert!(
        !dir.join("podcast_9.small.webp").is_file(),
        "no half-baked small file left behind"
    );
    root.mark_success();
}
