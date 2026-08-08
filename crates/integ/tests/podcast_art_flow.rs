//! Podcast artwork serving — `GET /podcasts/{id}/art`. Mirrors the media/audio
//! auth model: the credential travels in the `auth_media` cookie (or a normal
//! bearer for programmatic fetches), and only server-held files are served. The
//! integ tier had zero art coverage; these pin the three contract cases:
//!   - no credential                  → 401
//!   - authed but no `art_file_path`  → 204 (UI shows its placeholder)
//!   - authed + art staged on disk    → 200 + `Cache-Control: private, max-age=86400`
//!
//! The art file is staged on disk and `art_file_path` set directly (so
//! `ensure_podcast_art` short-circuits — no network egress), mirroring the
//! episodes art unit test in `routers/episodes/art.rs`.
//!
//! Run with: `cargo nextest run -p halogen-integ -E 'binary(podcast_art_flow)'`

use halogen_integ::*;
use sea_orm::{ActiveModelTrait, ActiveValue};

/// Stage a fake PNG inside the app's `media_root` (paths outside it are
/// refused by the confinement check) and set `art_file_path` on the podcast so
/// the art cache short-circuits to the staged file.
async fn stage_podcast_art(app: &TestApp, podcast_id: i32) -> std::path::PathBuf {
    std::fs::create_dir_all(app.media_root()).expect("create media_root");
    let art_path = app
        .media_root()
        .join(format!("podcast-art-{podcast_id}.png"));
    std::fs::write(&art_path, b"\x89PNG\r\n\x1a\nfake-podcast-art").expect("write art");

    let update = halogen_orm::podcast::ActiveModel {
        id: ActiveValue::set(podcast_id),
        art_file_path: ActiveValue::set(Some(art_path.display().to_string())),
        ..Default::default()
    };
    update.update(&app.dbc).await.expect("set art_file_path");
    art_path
}

#[tokio::test]
async fn podcast_art_no_credential_is_unauthorized() {
    let app = spawn().await;
    let podcast_id = app.seed_podcast("Arty", "https://feed.test/arty").await;
    let http = reqwest::Client::new();

    let resp = http
        .get(format!("{}/api/v1/podcasts/{podcast_id}/art", app.base_url))
        .send()
        .await
        .expect("art GET");
    assert_eq!(resp.status().as_u16(), 401, "no cookie/bearer → 401");
}

#[tokio::test]
async fn podcast_art_no_art_file_is_no_content() {
    let app = spawn().await;
    let admin = app.seed_admin().await;
    let podcast_id = app.seed_podcast("Arty", "https://feed.test/arty").await;
    let http = reqwest::Client::new();

    // Bearer authenticates; the seeded podcast has no art_url/art_file_path, so
    // the cache has nothing to fetch → 204 (not 404, so the browser doesn't log a
    // console error for the benign empty-art case; the UI shows its placeholder).
    let resp = http
        .get(format!("{}/api/v1/podcasts/{podcast_id}/art", app.base_url))
        .bearer_auth(&admin.token)
        .send()
        .await
        .expect("art GET");
    assert_eq!(
        resp.status().as_u16(),
        204,
        "no artwork → 204 (UI placeholder)"
    );
}

#[tokio::test]
async fn podcast_art_staged_on_disk_serves_with_cache_control() {
    let app = spawn().await;
    let admin = app.seed_admin().await;
    let podcast_id = app.seed_podcast("Arty", "https://feed.test/arty").await;
    let art_path = stage_podcast_art(&app, podcast_id).await;
    let http = reqwest::Client::new();

    // Cookie path (`<img src>`): a media-scoped cookie authorizes and serves the
    // staged bytes with the day-long browser-cache header.
    let media = app.media_jwt(&admin.id.to_string());
    let resp = http
        .get(format!("{}/api/v1/podcasts/{podcast_id}/art", app.base_url))
        .header(reqwest::header::COOKIE, format!("auth_media={media}"))
        .send()
        .await
        .expect("art GET");
    assert_eq!(resp.status().as_u16(), 200, "staged art streams");
    assert_eq!(
        resp.headers()
            .get(reqwest::header::CACHE_CONTROL)
            .and_then(|v| v.to_str().ok()),
        Some("private, max-age=86400"),
        "art carries the day-long private Cache-Control"
    );
    let body = resp.bytes().await.expect("art body");
    assert_eq!(
        body.as_ref(),
        b"\x89PNG\r\n\x1a\nfake-podcast-art",
        "served bytes match the staged file"
    );

    let _ = std::fs::remove_file(&art_path);
}
