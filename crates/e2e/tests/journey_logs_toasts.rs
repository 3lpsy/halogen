//! Journey F — device logs + toasts.
//!
//! Two client-only surfaces nothing else drives end to end:
//!   - **Device logs**: enable capture + pick a level on the viewer page
//!     (/logs/device), generate some log lines by navigating, reach the viewer
//!     again from the Settings menu ("Device Logs"), confirm it's capturing
//!     lines, filter via the search box, then Clear to the empty state.
//!   - **Toasts**: trigger a runtime error (a device download with the server
//!     unreachable) and assert the daisyUI toast (`.toast .alert-error`) carries
//!     the expected message.
//!
//! `#[ignore]` by default; run via `just test-e2e`.

use halogen_e2e::{
    body_text, browser_session, click, click_el, count, login_via_ui, patch_active_config,
    require_dist, run_session, wait_for_css, wait_for_text,
};
use halogen_integ::support;
use std::time::Duration;
use thirtyfour::Key;
use thirtyfour::error::WebDriverError;
use thirtyfour::prelude::*;

#[tokio::test]
#[ignore = "needs Chrome + chromedriver + a built dist/ (run via `just test-e2e`)"]
async fn logs_and_toasts_journey() {
    if !require_dist() {
        return;
    }

    let app = support::spawn_with(support::SpawnOptions {
        use_mock_download: true,
        ..Default::default()
    })
    .await;
    let admin = app.seed_admin().await;
    let podcast_id = app
        .seed_podcast("Main Show", "https://feed.test/main")
        .await;
    app.seed_episodes(podcast_id, 3).await;

    let Some((_driver_guard, driver)) = browser_session().await else {
        return;
    };

    run_session(driver, "logs / toasts journey", async |driver| {
        login_via_ui(&driver, &app.base_url, &admin.username, &admin.password).await;

        // ── Device Logs page: enable capture + set the level ────────────────
        // Capture config lives on the viewer page now (moved out of Settings).
        driver.goto(format!("{}/logs/device", app.base_url)).await?;
        assert!(
            wait_for_text(&driver, "Enable Device Logs", Duration::from_secs(10)).await,
            "device logs capture controls did not render; body:\n{}",
            body_text(&driver).await
        );
        // The capture checkbox (the controls section's `input.checkbox`).
        let enable = driver
            .query(By::XPath(
                "//div[.//label[normalize-space()='Enable Device Logs']]//input[@type='checkbox']",
            ))
            .first()
            .await?;
        if !enable.is_selected().await? {
            enable.click().await?;
        }
        // Pick a permissive level so navigation logs are captured.
        driver
            .query(By::XPath(
                "//div[label[normalize-space()='Log Level']]//select/option[@value='Debug']",
            ))
            .first()
            .await?
            .click()
            .await
            .ok();

        // The toggle's effect persists to localStorage and applies capture live
        // (`pages/logs` → `logging::set_enabled`). Belt-and-braces against any
        // race: force the persisted shape (merging onto existing config) and
        // reload — config-load re-applies it (`providers/config.rs`), so capture
        // is unambiguously on before we generate log lines.
        patch_active_config(
            &driver,
            "c.device_logs = Object.assign({}, c.device_logs, { enabled: true, level: 'Debug' });",
            Vec::new(),
        )
        .await?;
        driver.refresh().await?;

        // ── Generate some log lines by navigating ───────────────────────────
        driver.goto(format!("{}/latest", app.base_url)).await?;
        wait_for_css(&driver, "#episode-scroll h2", Duration::from_secs(10)).await;
        driver.goto(format!("{}/podcasts", app.base_url)).await?;
        wait_for_text(&driver, "Main Show", Duration::from_secs(10)).await;

        // ── Device logs viewer: capturing + lines present ───────────────────
        // Settings is a menu page now; the viewer is behind its "Device Logs" link.
        driver.goto(format!("{}/settings", app.base_url)).await?;
        wait_for_text(&driver, "Device Logs", Duration::from_secs(10)).await;
        click(&driver, "a[href='/logs/device']").await?;
        assert!(
            wait_for_text(&driver, "Device Logs", Duration::from_secs(10)).await,
            "device logs page did not render; body:\n{}",
            body_text(&driver).await
        );
        // "Capturing • N lines" proves capture is on and the ring has content.
        assert!(
            wait_for_text(&driver, "Capturing", Duration::from_secs(10)).await,
            "device logs header did not show the capturing state; body:\n{}",
            body_text(&driver).await
        );
        let lines_before = count(&driver, "div.font-mono > div").await;
        assert!(
            lines_before > 0,
            "expected captured log lines on the viewer; body:\n{}",
            body_text(&driver).await
        );

        // ── Search filter narrows the visible lines ─────────────────────────
        // A substring that won't match any real log line → empty-match state.
        let search = driver
            .query(By::Css("input[placeholder='Search logs…']"))
            .first()
            .await?;
        search.send_keys("zzz-no-such-log-zzz").await?;
        assert!(
            wait_for_text(
                &driver,
                "No logs match your search.",
                Duration::from_secs(10)
            )
            .await,
            "search filter did not narrow the log list; body:\n{}",
            body_text(&driver).await
        );

        // ── Clear → empty state ─────────────────────────────────────────────
        // Clear the search first so the empty message is the "no logs" variant.
        for _ in 0..20 {
            search.send_keys(Key::Backspace).await?;
        }
        // The capture controls (added to this page) make it tall enough that
        // focusing the search box scrolls the header — and its Clear button —
        // up under the fixed navbar. `click_el` scrolls it back into view and
        // falls back to a JS click if the navbar still intercepts.
        // Clear is now an icon button (label-less); target its aria-label.
        let clear_btn = driver
            .query(By::Css("button[aria-label='Clear']"))
            .first()
            .await?;
        click_el(&driver, &clear_btn).await?;
        assert!(
            wait_for_text(&driver, "No logs captured yet.", Duration::from_secs(10)).await,
            "Clear did not empty the log ring; body:\n{}",
            body_text(&driver).await
        );

        // ── Toast: an error toast on a failed action ────────────────────────
        // Cut the client off (dead port) so the next device download fails, which
        // raises an error toast through the shared toast funnel.
        patch_active_config(&driver, "c.server_url = 'http://127.0.0.1:1';", Vec::new()).await?;
        driver.refresh().await?;
        driver.goto(format!("{}/latest", app.base_url)).await?;
        assert!(
            wait_for_css(&driver, "#episode-scroll h2", Duration::from_secs(10)).await,
            "Latest did not render offline; body:\n{}",
            body_text(&driver).await
        );
        driver
            .query(By::Css("button[title='Download to device']"))
            .first()
            .await?
            .click()
            .await?;
        assert!(
            wait_for_css(&driver, ".toast .alert-error", Duration::from_secs(10)).await,
            "a failed device download should raise an error toast; body:\n{}",
            body_text(&driver).await
        );
        // The alert element can appear a tick before its text node populates, so
        // poll the text until it's non-empty rather than reading once and racing
        // the render (the toast now fires instantly, which exposed that race).
        let mut toast_text = String::new();
        for _ in 0..25 {
            toast_text = driver
                .query(By::Css(".toast .alert-error"))
                .first()
                .await?
                .text()
                .await
                .unwrap_or_default();
            if !toast_text.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        assert!(
            toast_text.contains("download"),
            "error toast text unexpected: '{toast_text}'"
        );

        Ok::<_, WebDriverError>(())
    })
    .await;
}
