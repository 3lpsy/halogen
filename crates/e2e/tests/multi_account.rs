//! Add a second account on the same server through Settings, verify isolated queue state across switches, then sign one
//! out. Each account uses separate IndexedDB storage. Ignored by default; run with `just test-e2e`.

use halogen_e2e::{
    body_text, browser_session, click, click_button_text, click_el, count, fill, login_via_ui,
    require_dist, run_session, wait_for_css, wait_for_text, wait_for_text_gone,
};
use halogen_integ::support;
use std::time::Duration;
use thirtyfour::error::WebDriverError;
use thirtyfour::prelude::*;

const ROW: &str = "#episode-scroll h2";

/// Fill + submit the login form already on screen (account B is added from
/// inside the app, which routes straight to `/auth/login`; the form's server
/// URL is already prefilled from the active account, so only the credentials
/// change — `login_via_ui`'s full onboarding path doesn't apply here).
async fn submit_login_form(driver: &thirtyfour::WebDriver, username: &str, password: &str) {
    assert!(
        wait_for_css(driver, "input[type='password']", Duration::from_secs(10)).await,
        "add-account login form never appeared; body:\n{}",
        body_text(driver).await
    );
    fill(driver, "input[type='text']", username)
        .await
        .expect("type username");
    fill(driver, "input[type='password']", password)
        .await
        .expect("type password");
    click(driver, "button[type='submit']")
        .await
        .expect("submit login");
}

#[tokio::test]
#[ignore = "needs Chrome + chromedriver + a built dist/ (run via `just test-e2e`)"]
async fn multi_account_isolation_switch_and_signout() {
    if !require_dist() {
        return;
    }

    let app = support::spawn().await;
    let admin = app.seed_admin().await; // account A
    app.seed_login_user("bob", "bobpw123").await; // account B (loginable)

    // Seed a queue-backing default playlist + episodes for A only, so B's empty
    // queue is a provable isolation signal.
    let podcast = app
        .seed_podcast("Main Show", "https://feed.test/main")
        .await;
    let episodes = app.seed_episodes(podcast, 5).await;
    let _ = &episodes;
    app.seed_playlist("Queue", true).await;

    let Some((_driver_guard, driver)) = browser_session().await else {
        return;
    };

    run_session(driver, "multi-account journey", async |driver| {
        // ── Account A: sign in and queue an episode ─────────────────────────
        login_via_ui(&driver, &app.base_url, &admin.username, &admin.password).await;

        driver.goto(format!("{}/latest", app.base_url)).await?;
        assert!(
            wait_for_css(&driver, ROW, Duration::from_secs(10)).await,
            "A: latest list never rendered; body:\n{}",
            body_text(&driver).await
        );
        // Add the first episode to A's queue via the row kebab.
        click(&driver, "button[aria-label='Episode actions']").await?;
        assert!(
            wait_for_text(&driver, "Add to queue", Duration::from_secs(5)).await,
            "A: kebab 'Add to queue' action missing; body:\n{}",
            body_text(&driver).await
        );
        click_button_text(&driver, "Add to queue").await?;

        driver.goto(format!("{}/queue", app.base_url)).await?;
        assert!(
            wait_for_css(&driver, ROW, Duration::from_secs(10)).await,
            "A: queued episode did not appear on the Queue; body:\n{}",
            body_text(&driver).await
        );

        // ── Add account B via Settings → Accounts → Add account ─────────────
        driver
            .goto(format!("{}/settings/accounts", app.base_url))
            .await?;
        assert!(
            wait_for_text(&driver, "Add account", Duration::from_secs(10)).await,
            "Accounts page 'Add account' missing; body:\n{}",
            body_text(&driver).await
        );
        click_button_text(&driver, "Add account").await?;
        submit_login_form(&driver, "bob", "bobpw123").await;
        assert!(
            wait_for_css(&driver, "a[href='/queue']", Duration::from_secs(10)).await,
            "B: never reached an authenticated view after add-account login; body:\n{}",
            body_text(&driver).await
        );

        // ── Isolation: B's queue is empty (A's queued row must NOT leak) ────
        driver.goto(format!("{}/queue", app.base_url)).await?;
        assert!(
            wait_for_text(&driver, "No queue yet", Duration::from_secs(10)).await,
            "B's queue should be empty (isolation) but wasn't; body:\n{}",
            body_text(&driver).await
        );
        assert_eq!(
            count(&driver, ROW).await,
            0,
            "B's queue must not show A's queued episode (cross-account leak); body:\n{}",
            body_text(&driver).await
        );
        // The device registry now holds two accounts, B active.
        driver
            .goto(format!("{}/settings/accounts", app.base_url))
            .await?;
        assert!(
            wait_for_text(&driver, "bob", Duration::from_secs(5)).await
                && body_text(&driver).await.contains(&admin.username),
            "Accounts list should show both users; body:\n{}",
            body_text(&driver).await
        );

        // Wait for the active-account registry write, then reload as A. This avoids racing the asynchronous subtree
        // remount and worker reauthentication while checking A's persisted queue.
        click_button_text(&driver, "Switch").await?;
        tokio::time::sleep(Duration::from_secs(2)).await;
        driver.refresh().await?;
        assert!(
            wait_for_css(&driver, "a[href='/queue']", Duration::from_secs(15)).await,
            "app chrome missing after switching back to A"
        );
        driver.goto(format!("{}/queue", app.base_url)).await?;
        assert!(
            wait_for_css(&driver, ROW, Duration::from_secs(20)).await,
            "A: queued episode did not survive the account switch (isolation broken); body:\n{}",
            body_text(&driver).await
        );

        // ── Sign out B: only A remains ──────────────────────────────────────
        driver
            .goto(format!("{}/settings/accounts", app.base_url))
            .await?;
        assert!(
            wait_for_text(&driver, "bob", Duration::from_secs(5)).await,
            "expected B's row on the accounts page before signing it out"
        );
        // Target B's OWN "Sign out" (both rows have one; A's is first in
        // document order): the first Sign-out button that follows B's username.
        const B_SIGNOUT: &str =
            "//span[normalize-space()='bob']/following::button[normalize-space()='Sign out'][1]";
        // Re-find each button before clicking and wait for row removal. Connection-state renders can detach targets,
        // and non-active sign-out persists asynchronously without a subtree remount.
        let mut signed_out = false;
        for _ in 0..5 {
            let Ok(b_signout) = driver.find(By::XPath(B_SIGNOUT)).await else {
                // No button. Either the row is already gone (a click landed) or
                // the markup moved — the absence check decides which, so a
                // missing button can never pass as success.
                signed_out = wait_for_text_gone(&driver, "bob", Duration::from_secs(3)).await;
                break;
            };
            click_el(&driver, &b_signout).await.ok();
            if wait_for_text_gone(&driver, "bob", Duration::from_secs(3)).await {
                signed_out = true;
                break;
            }
        }
        assert!(
            signed_out,
            "B's row should disappear from the list after signing it out; body:\n{}",
            body_text(&driver).await
        );
        // A must remain signed in (still on the accounts page / app chrome).
        assert!(
            wait_for_css(&driver, "a[href='/queue']", Duration::from_secs(10)).await,
            "A should remain signed in after signing out B; body:\n{}",
            body_text(&driver).await
        );
        assert!(
            body_text(&driver).await.contains(&admin.username),
            "A should still be listed after signing out B; body:\n{}",
            body_text(&driver).await
        );

        Ok::<_, WebDriverError>(())
    })
    .await;
}
