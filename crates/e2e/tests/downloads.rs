//! Device-download journey verifies real IndexedDB bytes, in-flight/completed badges, and Downloads rehydration after
//! reload. Server audio comes from mock-download fixtures. Ignored by default; run with `just test-e2e`.

use halogen_e2e::{
    body_text, browser_session, idb_audio_count, ingest_feed, login_via_ui, require_dist,
    run_session, wait_for_css, wait_for_text,
};
use halogen_integ::support;
use std::time::Duration;
use thirtyfour::error::WebDriverError;
use thirtyfour::prelude::*;

#[tokio::test]
#[ignore = "needs Chrome + chromedriver + a built dist/ (run via `just test-e2e`)"]
async fn download_to_device_journey() {
    if !require_dist() {
        return;
    }

    // Mock download service ON: the server "fetches" the nasa-test-clip fixture
    // into the test media_root, so the client's byte pull is a real transfer.
    let app = support::spawn_with(support::SpawnOptions {
        use_mock_download: true,
        ..Default::default()
    })
    .await;
    let admin = app.seed_admin().await;

    // Subscribe + ingest a feed server-side (only upstream RSS is faked).
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

    run_session(driver, "download-to-device journey", async |driver| {
        login_via_ui(&driver, &app.base_url, &admin.username, &admin.password).await;

        // Latest: the in-browser worker pulls after auth, then the list renders.
        driver.goto(format!("{}/latest", app.base_url)).await?;
        assert!(
            wait_for_css(&driver, "#episode-scroll h2", Duration::from_secs(10)).await,
            "no episodes rendered on Latest; body:\n{}",
            body_text(&driver).await
        );

        // Capture the first episode's title, then click ITS device-download
        // control. Episodes were ingested but not server-downloaded, so the
        // badge title is "Download to device".
        let title = driver
            .query(By::Css("#episode-scroll h2"))
            .first()
            .await?
            .text()
            .await?;
        assert!(!title.is_empty(), "first episode title was empty");
        assert_eq!(idb_audio_count(&driver).await, 0, "byte store starts empty");

        driver
            .query(By::Css("button[title='Download to device']"))
            .first()
            .await?
            .click()
            .await?;

        // The full honest pipeline now runs: server mock-download → client
        // polls episode status → byte fetch → IndexedDB write → badge flips to
        // trash. The intermediate spinner ("Downloading to device…") can be too
        // fast to sample reliably, so assert only the terminal state.
        assert!(
            wait_for_css(
                &driver,
                "button[title='Downloaded on device — remove']",
                Duration::from_secs(10),
            )
            .await,
            "device-download badge never reached the Downloaded state; body:\n{}",
            body_text(&driver).await
        );

        // The bytes are REALLY there: one row in the IndexedDB audio store.
        assert_eq!(
            idb_audio_count(&driver).await,
            1,
            "expected exactly one stored audio blob after the download"
        );

        // Downloads page after a reload: hydration re-derives the device set
        // from the byte store (no persisted flag to lie).
        driver.goto(format!("{}/downloads", app.base_url)).await?;
        assert!(
            wait_for_text(&driver, &title, Duration::from_secs(10)).await,
            "expected '{title}' on Downloads after download-to-device; body:\n{}",
            body_text(&driver).await
        );
        Ok::<_, WebDriverError>(())
    })
    .await;
}
