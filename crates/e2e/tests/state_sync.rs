//! Add to Queue through the kebab menu; verify the optimistic worker update appears immediately and rehydrates from
//! local storage after reload. Ignored by default; run with `just test-e2e`.

use halogen_e2e::{
    body_text, browser_session, click_button_text, login_via_ui, patch_active_config, require_dist,
    run_session, wait_for_count, wait_for_text,
};
use halogen_integ::support;
use std::time::Duration;
use thirtyfour::error::WebDriverError;
use thirtyfour::prelude::*;

const EPISODES: usize = 5;
const ROW: &str = "#episode-scroll h2";

#[tokio::test]
#[ignore = "needs Chrome + chromedriver + a built dist/ (run via `just test-e2e`)"]
async fn add_to_queue_via_menu_syncs_and_persists() {
    if !require_dist() {
        return;
    }

    let app = support::spawn().await;
    let admin = app.seed_admin().await;
    let podcast_id = app
        .seed_podcast("Main Show", "https://feed.test/main")
        .await;
    app.seed_episodes(podcast_id, EPISODES).await;
    // The queue resolves to the `is_default` playlist (`GET /playlists/default`),
    // so seed one marked default. It starts empty.
    app.seed_playlist("Queue", true).await;

    let Some((_driver_guard, driver)) = browser_session().await else {
        return;
    };

    run_session(driver, "state-sync queue journey", async |driver| {
        login_via_ui(&driver, &app.base_url, &admin.username, &admin.password).await;

        // Latest renders the synced episodes.
        driver.goto(format!("{}/latest", app.base_url)).await?;
        let n = wait_for_count(&driver, ROW, EPISODES, Duration::from_secs(10)).await;
        assert!(
            n >= EPISODES,
            "expected episodes on Latest before queueing; body:\n{}",
            body_text(&driver).await
        );

        // The first row's title — the one we'll add to the queue.
        let title = driver.query(By::Css(ROW)).first().await?.text().await?;
        assert!(!title.is_empty(), "first episode title was empty");

        // Open that row's kebab menu and pick "Add to queue".
        driver
            .query(By::Css("button[aria-label='Episode actions']"))
            .first()
            .await?
            .click()
            .await?;
        assert!(
            wait_for_text(&driver, "Add to queue", Duration::from_secs(5)).await,
            "quick menu did not open with an 'Add to queue' action; body:\n{}",
            body_text(&driver).await
        );
        click_button_text(&driver, "Add to queue").await?;

        // Let the worker apply the optimistic change + persist the playlist.
        tokio::time::sleep(Duration::from_secs(1)).await;

        // Queue now lists the episode (optimistic, in-memory).
        driver.goto(format!("{}/queue", app.base_url)).await?;
        assert!(
            wait_for_text(&driver, &title, Duration::from_secs(10)).await,
            "queued episode '{title}' did not appear on the Queue; body:\n{}",
            body_text(&driver).await
        );

        // Sever the API before reloading so the re-hydration can ONLY come from the local store — otherwise the
        // worker's pull re-fetches the queue from the server and the assertion passes via that echo even if
        // local persistence is broken (the bug this test claims to cover). Same dead-port repoint the offline
        // journey uses; token + setup stay.
        patch_active_config(&driver, "c.server_url = 'http://127.0.0.1:1';", Vec::new()).await?;

        // Reload from scratch: the Queue must re-hydrate the membership from the
        // local store, not depend on the live in-memory mutation OR a server pull.
        driver.refresh().await?;
        driver.goto(format!("{}/queue", app.base_url)).await?;
        assert!(
            wait_for_text(&driver, &title, Duration::from_secs(10)).await,
            "queued episode '{title}' did not survive a reload from the LOCAL store \
             (persistence broken); body:\n{}",
            body_text(&driver).await
        );

        Ok::<_, WebDriverError>(())
    })
    .await;
}
