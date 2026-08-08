//! Lazy-load migration guard — the views that moved off the old "pull everything
//! into memory" model now render via the offline-first/paged path:
//!   - **Podcasts** list (paged SWR from the store + server pages) shows the card
//!     and its server-computed `episode_count`.
//!   - **Podcast detail** (paged, `filter[podcast_id]`) shows the podcast's episodes.
//!   - **History** (id-list over the user's playbacks) resolves episode bodies by id.
//!   - A **deep-linked** episode that was never browsed loads via the on-miss
//!     `get_episode` fetch (no full hydrate any more).
//!
//! `#[ignore]` by default; run via `just test-e2e`.

use halogen_e2e::{
    body_text, browser_session, login_via_ui, require_dist, run_session, wait_for_css,
    wait_for_text,
};
use halogen_integ::support;
use std::time::Duration;
use thirtyfour::error::WebDriverError;

const EPISODES: usize = 12;

#[tokio::test]
#[ignore = "needs Chrome + chromedriver + a built dist/ (run via `just test-e2e`)"]
async fn lazy_lists_render_paged_and_deeplink_loads_on_miss() {
    if !require_dist() {
        return;
    }

    let app = support::spawn().await;
    let admin = app.seed_admin().await;
    let podcast_id = app
        .seed_podcast("Lazy Show", "https://feed.test/lazy")
        .await;
    // "Episode 0000" (newest) .. "Episode 0011" (oldest).
    let episode_ids = app.seed_episodes(podcast_id, EPISODES).await;
    // Give the first three a playback so the History list has rows to resolve.
    app.seed_playbacks(admin.id, &episode_ids[..3]).await;

    let Some((_driver_guard, driver)) = browser_session().await else {
        return;
    };

    run_session(driver, "lazy lists journey", async |driver| {
        login_via_ui(&driver, &app.base_url, &admin.username, &admin.password).await;

        // ── Deep-link on-miss (FIRST, while the pool is still empty) ──────────
        // Right after login nothing has populated the episode pool (no full
        // hydrate), so opening an episode directly must fetch it on demand.
        let deep_id = episode_ids[EPISODES - 1];
        let oldest = format!("Episode {:04}", EPISODES - 1);
        driver
            .goto(format!("{}/episodes/{deep_id}", app.base_url))
            .await?;
        assert!(
            wait_for_text(&driver, &oldest, Duration::from_secs(10)).await,
            "deep-linked episode did not load on miss; body:\n{}",
            body_text(&driver).await
        );

        // ── Podcasts list (paged SWR) ────────────────────────────────────────
        // The card renders from the server page, and its count comes from the
        // server-computed `episode_count` (not an in-memory episode scan).
        driver.goto(format!("{}/podcasts", app.base_url)).await?;
        assert!(
            wait_for_text(&driver, "Lazy Show", Duration::from_secs(10)).await,
            "podcasts list did not render the paged card; body:\n{}",
            body_text(&driver).await
        );
        assert!(
            wait_for_text(
                &driver,
                &format!("{EPISODES} episodes"),
                Duration::from_secs(10)
            )
            .await,
            "podcast card did not show the server episode_count; body:\n{}",
            body_text(&driver).await
        );

        // ── Podcast detail (paged, scoped by podcast_id) ─────────────────────
        driver
            .goto(format!("{}/podcasts/{podcast_id}", app.base_url))
            .await?;
        assert!(
            wait_for_css(&driver, "#episode-scroll h2", Duration::from_secs(10)).await,
            "podcast detail did not render paged episodes; body:\n{}",
            body_text(&driver).await
        );

        // ── History (id-list over playbacks) ─────────────────────────────────
        driver.goto(format!("{}/history", app.base_url)).await?;
        assert!(
            wait_for_css(&driver, "#episode-scroll h2", Duration::from_secs(10)).await,
            "history did not render any played episodes; body:\n{}",
            body_text(&driver).await
        );

        Ok::<_, WebDriverError>(())
    })
    .await;
}
