//! Browser smoke test: the embedded frontend boots in a real browser. `#[ignore]` by default — needs
//! Chrome/Chromium + `chromedriver` on PATH and a built `dist/`. Run via `just test-e2e`. The server serves the
//! bundled `dist/` from memory (embed-frontend), exactly like a release build.

use halogen_e2e::{body_text, browser_session, require_dist, run_session, wait_for_text};
use halogen_integ::support;
use std::time::Duration;
use thirtyfour::error::WebDriverError;

#[tokio::test]
#[ignore = "needs Chrome + chromedriver + a built dist/ (run via `just test-e2e`)"]
async fn frontend_boots_in_browser() {
    if !require_dist() {
        return;
    }

    // Real server serving the embedded frontend (the `/` fallback).
    let app = support::spawn().await;

    let Some((_driver_guard, driver)) = browser_session().await else {
        return;
    };

    run_session(driver, "smoke assertions", async |driver| {
        driver.goto(&app.base_url).await?;
        // Unauthenticated, the app boots and RootGuard redirects to the login
        // page (server URL + credentials on one form).
        let ok = wait_for_text(&driver, "Connect to your server", Duration::from_secs(10)).await;
        assert!(
            ok,
            "expected the login screen to render; body was:\n{}",
            body_text(&driver).await
        );
        Ok::<_, WebDriverError>(())
    })
    .await;
}
