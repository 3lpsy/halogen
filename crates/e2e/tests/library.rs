//! Library journey — a podcast subscribed + ingested on the server shows up in the browser UI after the in-app
//! sync worker pulls it. Exercises the whole loop: mocked RSS → server ingestion → in-browser worker → list
//! rendering. `#[ignore]` by default; run via `just test-e2e`.

use halogen_e2e::{
    body_text, browser_session, ingest_feed, login_via_ui, require_dist, run_session, wait_for_text,
};
use halogen_integ::support;
use std::time::Duration;
use thirtyfour::error::WebDriverError;

#[tokio::test]
#[ignore = "needs Chrome + chromedriver + a built dist/ (run via `just test-e2e`)"]
async fn library_journey() {
    if !require_dist() {
        return;
    }

    let app = support::spawn().await;
    let admin = app.seed_admin().await;

    // Subscribe to a mocked feed and ingest it server-side (only upstream faked).
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

    run_session(driver, "library journey", async |driver| {
        // login_via_ui waits for the authenticated app chrome before returning.
        login_via_ui(&driver, &app.base_url, &admin.username, &admin.password).await;

        // The in-browser sync worker pulls after auth; the Latest list should
        // then show an episode title from the ingested feed.
        driver.goto(format!("{}/latest", app.base_url)).await?;
        assert!(
            wait_for_text(&driver, "European Startup", Duration::from_secs(10)).await,
            "expected a synced episode title on Latest; body:\n{}",
            body_text(&driver).await
        );
        Ok::<_, WebDriverError>(())
    })
    .await;
}
