//! Open a 35-episode playlist from its card and scroll beyond the 30-row window, exercising cached ID resolution and
//! missing-episode fetches. Ignored by default; run with `just test-e2e`.

use halogen_e2e::{
    body_text, browser_session, click, login_via_ui, require_dist, run_session, scroll_to_count,
    wait_for_count, wait_for_css, wait_for_url,
};
use halogen_integ::support;
use std::time::Duration;
use thirtyfour::error::WebDriverError;

const EPISODES: usize = 35;
const ROW: &str = "#episode-scroll h2";

#[tokio::test]
#[ignore = "needs Chrome + chromedriver + a built dist/ (run via `just test-e2e`)"]
async fn playlist_detail_renders_membership_lazily() {
    if !require_dist() {
        return;
    }

    let app = support::spawn().await;
    let admin = app.seed_admin().await;
    let podcast_id = app
        .seed_podcast("Main Show", "https://feed.test/main")
        .await;
    let episode_ids = app.seed_episodes(podcast_id, EPISODES).await;

    // A non-default playlist holding every seeded episode, in order.
    let playlist_id = app.seed_playlist("Road Trip", false).await;
    app.seed_playlist_episodes(playlist_id, &episode_ids).await;

    let Some((_driver_guard, driver)) = browser_session().await else {
        return;
    };

    run_session(driver, "playlist detail journey", async |driver| {
        login_via_ui(&driver, &app.base_url, &admin.username, &admin.password).await;

        // ── /playlists cards → the playlist links to its detail ─────────────
        driver.goto(format!("{}/playlists", app.base_url)).await?;
        let link = format!("a[href$=\"/playlists/{playlist_id}\"]");
        assert!(
            wait_for_css(&driver, &link, Duration::from_secs(10)).await,
            "playlist card linking to /playlists/{playlist_id} never appeared. body:\n{}",
            body_text(&driver).await
        );
        click(&driver, &link).await?;

        // ── /playlists/:id — lazy, offline-first episode list ───────────────
        assert!(
            wait_for_url(
                &driver,
                &format!("/playlists/{playlist_id}"),
                Duration::from_secs(10)
            )
            .await,
            "clicking the card did not navigate to the detail. body:\n{}",
            body_text(&driver).await
        );
        let initial = wait_for_count(&driver, ROW, 11, Duration::from_secs(10)).await;
        assert!(
            initial > 10,
            "playlist detail showed only {initial} episodes — the id-window didn't \
             resolve from the pool. body:\n{}",
            body_text(&driver).await
        );

        // Scrolling grows the window (and fills missing ids) to the whole membership.
        let grown = scroll_to_count(&driver, ROW, EPISODES, Duration::from_secs(12)).await;
        assert_eq!(
            grown,
            EPISODES,
            "scrolling did not reveal all {EPISODES} playlist episodes (got {grown}); \
             the id-list window isn't growing/filling. body:\n{}",
            body_text(&driver).await
        );

        Ok::<_, WebDriverError>(())
    })
    .await;
}
