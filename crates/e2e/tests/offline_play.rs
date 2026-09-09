//! Download real bytes into IndexedDB, reload against an unreachable server, and verify the downloaded episode remains
//! playable from a blob URL. Ignored by default; run with `just test-e2e`.

use halogen_e2e::{
    body_text, browser_session, ingest_feed, login_via_ui, patch_active_config, require_dist,
    run_session, wait_for_css,
};
use halogen_integ::support;
use std::time::Duration;
use thirtyfour::error::WebDriverError;
use thirtyfour::prelude::*;

#[tokio::test]
#[ignore = "needs Chrome + chromedriver + a built dist/ (run via `just test-e2e`)"]
async fn device_downloaded_episode_plays_offline() {
    if !require_dist() {
        return;
    }

    let app = support::spawn_with(support::SpawnOptions {
        use_mock_download: true,
        ..Default::default()
    })
    .await;
    let admin = app.seed_admin().await;

    let _feed = ingest_feed(
        &app,
        &admin.token,
        "Software Engineering Daily",
        "sed_podcast.xml",
    )
    .await;

    let Some((_driver_guard, driver)) = browser_session().await else {
        return;
    };

    run_session(driver, "offline playback journey", async |driver| {
        login_via_ui(&driver, &app.base_url, &admin.username, &admin.password).await;

        // Download the first episode to the device (full byte pipeline).
        driver.goto(format!("{}/latest", app.base_url)).await?;
        assert!(
            wait_for_css(&driver, "#episode-scroll h2", Duration::from_secs(10)).await,
            "no episodes rendered on Latest; body:\n{}",
            body_text(&driver).await
        );
        driver
            .query(By::Css("button[title='Download to device']"))
            .first()
            .await?
            .click()
            .await?;
        assert!(
            wait_for_css(
                &driver,
                "button[title='Downloaded on device — remove']",
                Duration::from_secs(10),
            )
            .await,
            "device download never completed; body:\n{}",
            body_text(&driver).await
        );

        // Kill connectivity: repoint the configured server at a dead port and
        // reload (token + setup flag survive — only the network is gone).
        patch_active_config(&driver, "c.server_url = 'http://127.0.0.1:1';", Vec::new()).await?;
        driver.refresh().await?;
        driver.goto(format!("{}/downloads", app.base_url)).await?;

        // The downloaded episode renders offline (hydrated from the byte store)
        // and its play badge is ENABLED — a device copy plays without a server.
        assert!(
            wait_for_css(&driver, "#episode-scroll h2", Duration::from_secs(10)).await,
            "downloaded episode missing on /downloads offline; body:\n{}",
            body_text(&driver).await
        );
        let play = driver
            .query(By::Css(".badge.badge-outline.cursor-pointer"))
            .first()
            .await?;
        play.click().await?;

        // Playback actually starts from local bytes: mini player appears and
        // its toggle reaches the Playing state (aria-label flips to "Pause").
        assert!(
            wait_for_css(&driver, "#mini-player", Duration::from_secs(10)).await,
            "mini player never appeared after offline play; body:\n{}",
            body_text(&driver).await
        );
        assert!(
            wait_for_css(
                &driver,
                "#mini-player button[aria-label='Pause']",
                Duration::from_secs(10),
            )
            .await,
            "offline playback never reached Playing; body:\n{}",
            body_text(&driver).await
        );

        Ok::<_, WebDriverError>(())
    })
    .await;
}
