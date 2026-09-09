//! Onboarding journey — the full first-run path, end to end through the real stack: wasm UI → API client → axum
//! server. load app → redirected to the login page → server URL + credentials on one form (health check, then
//! login) → land in the app (/ → /queue). `#[ignore]` by default; run via `just test-e2e`.

use halogen_e2e::{
    body_text, browser_session, login_via_ui, require_dist, run_session, wait_for_css, wait_for_url,
};
use halogen_integ::support;
use std::time::Duration;
use thirtyfour::error::WebDriverError;

#[tokio::test]
#[ignore = "needs Chrome + chromedriver + a built dist/ (run via `just test-e2e`)"]
async fn onboarding_journey() {
    if !require_dist() {
        return;
    }

    let app = support::spawn().await;
    let admin = app.seed_admin().await;

    let Some((_driver_guard, driver)) = browser_session().await else {
        return;
    };

    run_session(driver, "onboarding journey", async |driver| {
        // One connect form → app. `/` redirects to the Queue.
        login_via_ui(&driver, &app.base_url, &admin.username, &admin.password).await;

        assert!(
            wait_for_url(&driver, "/queue", Duration::from_secs(10)).await,
            "expected redirect to Queue after login; body was:\n{}",
            body_text(&driver).await
        );

        // App chrome is mounted: the Queue nav link is present.
        assert!(
            wait_for_css(&driver, "a[href='/queue']", Duration::from_secs(5)).await,
            "expected app navigation (Queue link)"
        );
        Ok::<_, WebDriverError>(())
    })
    .await;
}
