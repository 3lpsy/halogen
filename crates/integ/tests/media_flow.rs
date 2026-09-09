//! Use raw reqwest to verify exact audio bytes, Range/206 behavior, cookie auth, and 401/404 outcomes after ApiClient
//! setup. Also cover artwork through the same media-auth path. Run the halogen-integ media_flow binary.

use halogen_integ::*;
use sea_orm::{ActiveModelTrait, ActiveValue};

/// The audio URL — auth travels in the `auth_media` cookie, not the URL.
fn audio_url(app: &TestApp, episode_id: i32) -> String {
    format!("{}/api/v1/episodes/{episode_id}/audio", app.base_url)
}

#[tokio::test]
async fn media_streaming_journey() {
    let app = spawn().await;
    let admin = app.seed_admin().await;
    let client = api(&app, &admin.token);
    let http = reqwest::Client::new();
    // The media cookie value — a media-scoped token for the admin user.
    let media = app.media_jwt(&admin.id.to_string());
    let cookie = format!("auth_media={media}");

    // Arrange: ingest an episode (created as NOT_DOWNLOADED).
    let (_podcast_id, episode_id) = subscribe_and_poll(&app, &client, "sed_podcast.xml").await;

    // 1) Not downloaded yet → 404, even with a valid media cookie.
    let resp = http
        .get(audio_url(&app, episode_id))
        .header(reqwest::header::COOKIE, &cookie)
        .send()
        .await
        .expect("audio GET");
    assert_eq!(
        resp.status().as_u16(),
        404,
        "not-downloaded episode must 404"
    );

    // 2) Stage the episode's audio *inside* media_root and mark it downloaded.
    //  The write API no longer accepts a client-supplied `content_file_path`
    //  (that was an arbitrary-file-read vector), so this seeds the
    //  server-managed download state directly, as a real download would.
    let payload: &[u8] = b"HALOGEN-TEST-AUDIO-0123456789";
    app.stage_downloaded_audio(episode_id, payload).await;

    // 3) Now it streams: 200 with the exact file bytes.
    let resp = http
        .get(audio_url(&app, episode_id))
        .header(reqwest::header::COOKIE, &cookie)
        .send()
        .await
        .expect("audio GET");
    assert_eq!(
        resp.status().as_u16(),
        200,
        "downloaded episode must stream"
    );
    let body = resp.bytes().await.expect("audio body");
    assert_eq!(body.as_ref(), payload, "served bytes must match the file");

    // 4) Range support (seek/resume): a partial request yields 206 + the slice.
    let resp = http
        .get(audio_url(&app, episode_id))
        .header(reqwest::header::COOKIE, &cookie)
        .header(reqwest::header::RANGE, "bytes=0-3")
        .send()
        .await
        .expect("audio range GET");
    assert_eq!(
        resp.status().as_u16(),
        206,
        "range request must be Partial Content"
    );
    let body = resp.bytes().await.expect("range body");
    assert_eq!(
        body.as_ref(),
        &payload[0..4],
        "range must return the first 4 bytes"
    );

    // 5) A bad cookie is rejected (auth happens before file access).
    let resp = http
        .get(audio_url(&app, episode_id))
        .header(reqwest::header::COOKIE, "auth_media=not-a-valid-jwt")
        .send()
        .await
        .expect("audio GET bad cookie");
    assert_eq!(
        resp.status().as_u16(),
        401,
        "invalid cookie must be unauthorized"
    );

    // 6) No cookie at all → 401.
    let resp = http
        .get(audio_url(&app, episode_id))
        .send()
        .await
        .expect("audio GET no cookie");
    assert_eq!(
        resp.status().as_u16(),
        401,
        "missing cookie must be unauthorized"
    );
}

/// Episode artwork serving — `GET /episodes/{id}/art`. Same media-auth + cache
/// contract as podcast art (the integ tier had zero art coverage). Three cases:
/// 401 (no credential), 204 (no `art_file_path`), 200 + day-long `Cache-Control`
/// when art is staged on disk.
#[tokio::test]
async fn episode_art_serving() {
    let app = spawn().await;
    let admin = app.seed_admin().await;
    let http = reqwest::Client::new();

    let podcast_id = app.seed_podcast("Art Show", "https://feed.test/art").await;
    let episode_id = app
        .seed_episode(
            podcast_id,
            "Ep",
            "",
            chrono::Utc::now(),
            halogen_wire::PlaybackStatus::Unplayed,
        )
        .await;
    let art_url = format!("{}/api/v1/episodes/{episode_id}/art", app.base_url);

    // 1) No cookie/bearer → 401.
    let resp = http.get(&art_url).send().await.expect("art GET");
    assert_eq!(resp.status().as_u16(), 401, "no credential → 401");

    // 2) Authed, but the episode has no art_file_path → 204 (not 404, so the
    //  benign empty-art case stays out of the browser console).
    let resp = http
        .get(&art_url)
        .bearer_auth(&admin.token)
        .send()
        .await
        .expect("art GET");
    assert_eq!(resp.status().as_u16(), 204, "no artwork → 204");

    // 3) Stage an art file on disk + set art_file_path so the cache
    //  short-circuits; the bytes serve with the day-long Cache-Control.
    // Art must live inside media_root — the art endpoint confines served paths.
    let art_dir = app.media_root().join("art");
    std::fs::create_dir_all(&art_dir).expect("create art dir");
    let art_path = art_dir.join(format!("episode-{episode_id}.png"));
    std::fs::write(&art_path, b"\x89PNG\r\n\x1a\nfake-episode-art").expect("write art");
    let update = halogen_orm::episode::ActiveModel {
        id: ActiveValue::set(episode_id),
        art_file_path: ActiveValue::set(Some(art_path.display().to_string())),
        ..Default::default()
    };
    update.update(&app.dbc).await.expect("set art_file_path");

    let resp = http
        .get(&art_url)
        .bearer_auth(&admin.token)
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
        b"\x89PNG\r\n\x1a\nfake-episode-art",
        "served bytes match the staged file"
    );

    let _ = std::fs::remove_file(&art_path);
}
