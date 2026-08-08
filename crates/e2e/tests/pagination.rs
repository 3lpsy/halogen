//! Pagination + infinite-scroll journey across every list route.
//!
//! Regression for two coupled bugs:
//!   1. The sync worker pulled only the first server page (default size 10), so
//!      every list silently capped at 10 items regardless of library size.
//!   2. The episode lists virtualize 30 rows at a time and grow as a sentinel
//!      scrolls into view — useless if the data never exceeds the first page.
//!
//! We seed 35 episodes (> one virtualization window) plus 15 podcasts and 15
//! playlists, then assert each route shows far more than the old 10-item cap and
//! that scrolling the episode list reveals the whole set.
//!
//! `#[ignore]` by default; run via `just test-e2e`.

use halogen_e2e::{
    body_text, browser_session, idb_seed_audio, login_via_ui, require_dist, run_session,
    scroll_to_count, wait_for_count,
};
use halogen_integ::support;
use std::time::Duration;
use thirtyfour::error::WebDriverError;

const EPISODES: usize = 35;
const PODCASTS: usize = 15;
const PLAYLISTS: usize = 15;
const ROW: &str = "#episode-scroll h2";

#[tokio::test]
#[ignore = "needs Chrome + chromedriver + a built dist/ (run via `just test-e2e`)"]
async fn pagination_and_infinite_scroll() {
    if !require_dist() {
        return;
    }

    let app = support::spawn().await;
    let admin = app.seed_admin().await;

    // One podcast carries the bulk of the episodes; the rest pad the podcast list.
    let podcast_id = app
        .seed_podcast("Main Show", "https://feed.test/main")
        .await;
    let episode_ids = app.seed_episodes(podcast_id, EPISODES).await;

    // The queue resolves to the `is_default` playlist; seed one holding every
    // episode, so /queue has a full list too.
    let queue_id = app.seed_playlist("Queue", true).await;
    app.seed_playlist_episodes(queue_id, &episode_ids).await;

    // History keeps episodes that have a playback — give all of them one.
    app.seed_playbacks(admin.id, &episode_ids).await;

    // Pad podcasts and playlists past the old 10-item cap.
    for i in 1..PODCASTS {
        app.seed_podcast(&format!("Show {i:02}"), &format!("https://feed.test/{i}"))
            .await;
    }
    for i in 1..PLAYLISTS {
        app.seed_playlist(&format!("Playlist {i:02}"), false).await;
    }

    let Some((_driver_guard, driver)) = browser_session().await else {
        return;
    };

    run_session(
        driver,
        "pagination + infinite-scroll journey",
        async |driver| {
            login_via_ui(&driver, &app.base_url, &admin.username, &admin.password).await;

            // ── /latest — the full infinite-scroll proof ────────────────────────
            driver.goto(format!("{}/latest", app.base_url)).await?;
            let initial = wait_for_count(&driver, ROW, 11, Duration::from_secs(10)).await;
            assert!(
                initial > 10,
                "Latest showed only {initial} episodes — the sync worker capped at the \
             first server page. body:\n{}",
                body_text(&driver).await
            );
            // Scroll the sentinel into view to grow the virtualization window to the
            // whole seeded set.
            let grown = scroll_to_count(&driver, ROW, EPISODES, Duration::from_secs(12)).await;
            assert_eq!(
                grown,
                EPISODES,
                "infinite scroll did not reveal all {EPISODES} episodes (got {grown}); \
             the sentinel observer is not growing the list. body:\n{}",
                body_text(&driver).await
            );

            // ── /queue — episodes via the default playlist ──────────────────────
            driver.goto(format!("{}/queue", app.base_url)).await?;
            let n = wait_for_count(&driver, ROW, 11, Duration::from_secs(10)).await;
            assert!(
                n > 10,
                "Queue showed only {n} episodes; body:\n{}",
                body_text(&driver).await
            );

            // ── /history — episodes with a playback ─────────────────────────────
            driver.goto(format!("{}/history", app.base_url)).await?;
            let n = wait_for_count(&driver, ROW, 11, Duration::from_secs(10)).await;
            assert!(
                n > 10,
                "History showed only {n} episodes; body:\n{}",
                body_text(&driver).await
            );

            // ── /downloads — device-download set (seeded into the byte store) ────
            // Device downloads are device-local and their ground truth is the
            // IndexedDB byte store (`halogen.media.{segment}`/`audio`) — hydration derives
            // the set from the blobs present, so seed tiny blobs directly, then
            // reload so the worker re-hydrates from them.
            idb_seed_audio(&driver, &episode_ids).await?;
            driver.refresh().await?;
            driver.goto(format!("{}/downloads", app.base_url)).await?;
            let n = wait_for_count(&driver, ROW, 11, Duration::from_secs(10)).await;
            assert!(
                n > 10,
                "Downloads showed only {n} episodes; body:\n{}",
                body_text(&driver).await
            );

            // ── /podcasts — its own card list (no virtualization) ───────────────
            driver.goto(format!("{}/podcasts", app.base_url)).await?;
            let n =
                wait_for_count(&driver, "#podcast-scroll h2", 11, Duration::from_secs(10)).await;
            assert!(
                n > 10,
                "Podcasts showed only {n} cards; body:\n{}",
                body_text(&driver).await
            );

            // ── /playlists — its own card list ──────────────────────────────────
            driver.goto(format!("{}/playlists", app.base_url)).await?;
            let n =
                wait_for_count(&driver, "#playlist-scroll h2", 11, Duration::from_secs(10)).await;
            assert!(
                n > 10,
                "Playlists showed only {n} cards; body:\n{}",
                body_text(&driver).await
            );

            Ok::<_, WebDriverError>(())
        },
    )
    .await;
}
