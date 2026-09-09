//! Download real episode bytes to IndexedDB, switch the client to an unreachable server, play the stored audio offline,
//! then remove it and verify absence after reload. Ignored by default; run with `just test-e2e`.

use halogen_e2e::{
    body_text, browser_session, idb_audio_count, ingest_feed, login_via_ui, patch_active_config,
    require_dist, run_session, wait_for_css,
};
use halogen_integ::support;
use std::time::Duration;
use thirtyfour::error::WebDriverError;
use thirtyfour::prelude::*;

#[tokio::test]
#[ignore = "needs Chrome + chromedriver + a built dist/ (run via `just test-e2e`)"]
async fn download_offline_remove_journey() {
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

    run_session(
        driver,
        "download / offline / remove journey",
        async |driver| {
            login_via_ui(&driver, &app.base_url, &admin.username, &admin.password).await;

            // ── Download the first /latest episode to the device ────────────────
            driver.goto(format!("{}/latest", app.base_url)).await?;
            assert!(
                wait_for_css(&driver, "#episode-scroll h2", Duration::from_secs(10)).await,
                "no episodes on Latest; body:\n{}",
                body_text(&driver).await
            );
            assert_eq!(idb_audio_count(&driver).await, 0, "byte store starts empty");

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
            assert_eq!(
                idb_audio_count(&driver).await,
                1,
                "expected exactly one stored audio blob after the download"
            );

            // ── Go offline: dead port + reload ──────────────────────────────────
            patch_active_config(&driver, "c.server_url = 'http://127.0.0.1:1';", Vec::new())
                .await?;
            driver.refresh().await?;
            driver.goto(format!("{}/downloads", app.base_url)).await?;

            // The downloaded row renders offline; its play badge is enabled.
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

            // Plays from local bytes: mini player reaches Playing with no server.
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

            // Dismiss the player so it doesn't overlap the row controls.
            driver
                .query(By::Css("#mini-player button[aria-label='Close player']"))
                .first()
                .await?
                .click()
                .await?;

            // ── Remove the device download (per-row trash badge) ────────────────
            driver
                .query(By::Css("button[title='Downloaded on device — remove']"))
                .first()
                .await?
                .click()
                .await?;
            for _ in 0..40 {
                if idb_audio_count(&driver).await == 0 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
            assert_eq!(
                idb_audio_count(&driver).await,
                0,
                "removing the device download should empty the byte store"
            );

            // After a reload the /downloads list re-hydrates from the (now empty)
            // byte store — the row is gone.
            driver.refresh().await?;
            driver.goto(format!("{}/downloads", app.base_url)).await?;
            tokio::time::sleep(Duration::from_secs(1)).await;
            assert!(
                driver.find(By::Css("#episode-scroll h2")).await.is_err(),
                "removed download should not reappear on /downloads after reload; body:\n{}",
                body_text(&driver).await
            );

            Ok::<_, WebDriverError>(())
        },
    )
    .await;
}
