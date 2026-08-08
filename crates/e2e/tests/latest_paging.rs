//! `/latest` server-side pagination journey.
//!
//! `/latest` no longer holds the whole library in memory: it pages from the
//! server offline-first (cache → revalidate), fetching the next page when the
//! sentinel scrolls into view. This seeds well over two server pages (page size
//! is 30) so the scroll has to cross multiple server-page boundaries to reveal
//! the whole set — proving pages are fetched and accumulated, not loaded at once.
//!
//! `#[ignore]` by default; run via `just test-e2e`.

use halogen_e2e::{
    body_text, browser_session, login_via_ui, require_dist, run_session, scroll_to_count,
    wait_for_count,
};
use halogen_integ::support;
use std::time::Duration;
use thirtyfour::error::WebDriverError;

/// 65 = three server pages of 30 (30 + 30 + 5) — forces multi-page fetching.
const EPISODES: usize = 65;
const ROW: &str = "#episode-scroll h2";

#[tokio::test]
#[ignore = "needs Chrome + chromedriver + a built dist/ (run via `just test-e2e`)"]
async fn latest_fetches_multiple_server_pages_on_scroll() {
    if !require_dist() {
        return;
    }

    let app = support::spawn().await;
    let admin = app.seed_admin().await;
    let podcast_id = app
        .seed_podcast("Main Show", "https://feed.test/main")
        .await;
    app.seed_episodes(podcast_id, EPISODES).await;

    let Some((_driver_guard, driver)) = browser_session().await else {
        return;
    };

    run_session(driver, "latest server-paging journey", async |driver| {
        login_via_ui(&driver, &app.base_url, &admin.username, &admin.password).await;

        driver.goto(format!("{}/latest", app.base_url)).await?;

        // First server page renders without scrolling.
        let initial = wait_for_count(&driver, ROW, 11, Duration::from_secs(10)).await;
        assert!(
            initial > 10,
            "Latest showed only {initial} episodes on first load. body:\n{}",
            body_text(&driver).await
        );

        // Scrolling must walk across several server pages to reveal every episode —
        // 65 episodes is three pages of 30, so this only passes if each scroll
        // fetches the next page and accumulates it.
        let grown = scroll_to_count(&driver, ROW, EPISODES, Duration::from_secs(15)).await;
        assert_eq!(
            grown,
            EPISODES,
            "scrolling did not fetch+accumulate all {EPISODES} episodes (got {grown}); \
             multi-page server fetching is broken. body:\n{}",
            body_text(&driver).await
        );

        Ok::<_, WebDriverError>(())
    })
    .await;
}
