//! Navigation journey — after authenticating, click through the primary nav
//! chrome and assert each route loads. Steps build on each other (one session).
//!
//! `#[ignore]` by default; run via `just test-e2e`.

use halogen_e2e::{
    body_text, browser_session, click, login_via_ui, require_dist, run_session, wait_for_url,
};
use halogen_integ::support;
use std::time::Duration;
use thirtyfour::error::WebDriverError;

#[tokio::test]
#[ignore = "needs Chrome + chromedriver + a built dist/ (run via `just test-e2e`)"]
async fn navigation_journey() {
    if !require_dist() {
        return;
    }

    let app = support::spawn().await;
    let admin = app.seed_admin().await;

    let Some((_driver_guard, driver)) = browser_session().await else {
        return;
    };

    run_session(driver, "navigation journey", async |driver| {
        login_via_ui(&driver, &app.base_url, &admin.username, &admin.password).await;
        // `/` redirects to the Queue after login.
        assert!(
            wait_for_url(&driver, "/queue", Duration::from_secs(10)).await,
            "expected redirect to Queue after login; body:\n{}",
            body_text(&driver).await
        );

        // The default nav: Queue, Latest, Playlists, Downloads, History, Settings.
        for href in [
            "/queue",
            "/latest",
            "/playlists",
            "/downloads",
            "/history",
            "/settings",
        ] {
            click(&driver, &format!("a[href='{href}']")).await?;
            assert!(
                wait_for_url(&driver, href, Duration::from_secs(10)).await,
                "clicking nav '{href}' did not navigate there; body:\n{}",
                body_text(&driver).await
            );
        }
        Ok::<_, WebDriverError>(())
    })
    .await;
}
