//! Exercise multiselect entry/exit, selection counts, Selected filtering, and the bulk server-download action in one
//! browser session. Ignored by default; run with `just test-e2e`.

use halogen_e2e::{
    body_text, browser_session, click, click_button_text, click_el, count, first_text,
    login_via_ui, require_dist, run_session, wait_for_count_eq, wait_for_text,
};
use halogen_integ::support;
use std::time::Duration;
use thirtyfour::error::WebDriverError;
use thirtyfour::prelude::*;

/// One server page of episodes; newest ("Episode 0000") on top.
const EPISODES: usize = 6;

const ROW: &str = "#episode-scroll h2";
const ROW_CHECKBOX: &str = "#episode-scroll input[type='checkbox']";

#[tokio::test]
#[ignore = "needs Chrome + chromedriver + a built dist/ (run via `just test-e2e`)"]
async fn multiselect_bulk_actions() {
    if !require_dist() {
        return;
    }

    let app = support::spawn().await;
    let admin = app.seed_admin().await;
    let podcast_id = app
        .seed_podcast("Bulk Show", "https://feed.test/bulk")
        .await;
    let _episode_ids = app.seed_episodes(podcast_id, EPISODES).await;

    let Some((_driver_guard, driver)) = browser_session().await else {
        return;
    };

    run_session(driver, "multiselect journey", async |driver| {
        login_via_ui(&driver, &app.base_url, &admin.username, &admin.password).await;
        driver.goto(format!("{}/latest", app.base_url)).await?;

        // Baseline: all episodes on one page; no row checkboxes until multiselect.
        let initial = wait_for_count_eq(&driver, ROW, EPISODES, Duration::from_secs(10)).await;
        assert_eq!(
            initial,
            EPISODES,
            "expected {EPISODES} rows on first load; body:\n{}",
            body_text(&driver).await
        );
        assert_eq!(
            count(&driver, ROW_CHECKBOX).await,
            0,
            "no row checkboxes before entering multiselect"
        );

        // ── ENTER MULTISELECT ───────────────────────────────────────────────
        click(&driver, "button[aria-label='Select multiple']").await?;
        let boxes =
            wait_for_count_eq(&driver, ROW_CHECKBOX, EPISODES, Duration::from_secs(10)).await;
        assert_eq!(
            boxes, EPISODES,
            "every visible row should show a selection checkbox in multiselect"
        );

        // ── SELECT TWO ──────────────────────────────────────────────────────
        // Re-query before each click: ticking a box re-renders the list, which can
        // stale an element handle grabbed before the render.
        let checkboxes = driver.find_all(By::Css(ROW_CHECKBOX)).await?;
        click_el(&driver, &checkboxes[0]).await?;
        let checkboxes = driver.find_all(By::Css(ROW_CHECKBOX)).await?;
        click_el(&driver, &checkboxes[1]).await?;
        // The bulk-actions badge now reads "2" (its leading count span).
        let badge = driver
            .query(By::Css("button[aria-label='Bulk actions']"))
            .first()
            .await?;
        assert_eq!(
            badge.text().await.unwrap_or_default().trim(),
            "2",
            "the bulk-actions badge should show the selected count"
        );

        // ── SELECTED VIEW ───────────────────────────────────────────────────
        // The "Selected" chip restricts the list to the two ticked rows.
        let selected_xpath = "//button[normalize-space()='Selected']";
        let selected_chip = driver.query(By::XPath(selected_xpath)).first().await?;
        click_el(&driver, &selected_chip).await?;
        let only_selected = wait_for_count_eq(&driver, ROW, 2, Duration::from_secs(10)).await;
        assert_eq!(
            only_selected,
            2,
            "the Selected chip should show only the 2 ticked rows; body:\n{}",
            body_text(&driver).await
        );
        assert_eq!(
            first_text(&driver, ROW).await,
            "Episode 0000",
            "the newest selected row stays on top under the Selected view"
        );

        // Toggle it off → the full list returns (selection survives). Re-query the
        // chip: the list re-render between toggles can stale the handle.
        let selected_chip = driver.query(By::XPath(selected_xpath)).first().await?;
        click_el(&driver, &selected_chip).await?;
        let restored = wait_for_count_eq(&driver, ROW, EPISODES, Duration::from_secs(10)).await;
        assert_eq!(
            restored, EPISODES,
            "toggling Selected off restores the full list"
        );

        // ── BULK MENU + ACTION ──────────────────────────────────────────────
        // The badge opens the bulk menu. It offers add-to-playlist plus the
        // download options (the queue items are now real, not disabled stubs).
        click(&driver, "button[aria-label='Bulk actions']").await?;
        assert!(
            wait_for_text(&driver, "Download on server", Duration::from_secs(5)).await,
            "the bulk menu should offer the server download options"
        );
        let menu = body_text(&driver).await;
        assert!(
            !menu.contains("unimplemented"),
            "the bulk menu should no longer carry disabled placeholder items; body:\n{menu}"
        );
        assert!(
            menu.contains("Download to device"),
            "the bulk menu should offer the device download options; body:\n{menu}"
        );

        // Fire the server bulk download → the worker toasts the count.
        click_button_text(&driver, "Download on server").await?;
        assert!(
            wait_for_text(
                &driver,
                "Downloading 2 episodes on the server",
                Duration::from_secs(10),
            )
            .await,
            "a count toast should confirm the bulk server download; body:\n{}",
            body_text(&driver).await
        );

        Ok::<_, WebDriverError>(())
    })
    .await;
}
