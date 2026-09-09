//! Exercise `/latest` sort order, substring search/clear, and Unplayed/In Progress/Finished filters in one ordered
//! browser session. Ignored by default; run with `just test-e2e`.

use halogen_e2e::{
    body_text, browser_session, click, click_el, first_text, login_via_ui, require_dist,
    run_session, wait_for_count_eq, wait_for_css,
};
use halogen_integ::support;
use halogen_wire::PlaybackStatus;
use std::time::Duration;
use thirtyfour::error::WebDriverError;
use thirtyfour::prelude::*;

/// 12 episodes (one server page) with a clean three-way listen-state split: the
/// first 4 ids are forced `Finished`, the next 3 `Played` (in progress), and the
/// remaining 5 stay `Unplayed` (no status row).
const EPISODES: usize = 12;
const FINISHED: usize = 4;
const IN_PROGRESS: usize = 3;
const UNPLAYED: usize = EPISODES - FINISHED - IN_PROGRESS;

const ROW: &str = "#episode-scroll h2";

/// Open a list-bar dropdown by its trigger's `aria-label`. Sort and Filter are now
/// icon-only `dropdown` triggers (no visible text), so they're identified by
/// `aria-label` ("Sort" / "Filter"), not by trigger content.
async fn open_dropdown(driver: &WebDriver, trigger: &str) -> WebDriverResult<()> {
    let xpath = format!("//div[@role='button'][@aria-label='{trigger}']");
    let el = driver.query(By::XPath(xpath)).first().await?;
    click_el(driver, &el).await
}

/// Open the filter dropdown (re-focusing it each call so a re-render can't leave a
/// hidden checkbox), then toggle the chip whose label is `label`.
async fn toggle_chip(driver: &WebDriver, label: &str) -> WebDriverResult<()> {
    open_dropdown(driver, "Filter").await?;
    let xpath = format!("//label[.//span[normalize-space()='{label}']]//input[@type='checkbox']");
    let checkbox = driver.query(By::XPath(xpath)).first().await?;
    click_el(driver, &checkbox).await?;
    Ok(())
}

#[tokio::test]
#[ignore = "needs Chrome + chromedriver + a built dist/ (run via `just test-e2e`)"]
async fn list_controls_sort_search_filter() {
    if !require_dist() {
        return;
    }

    let app = support::spawn().await;
    let admin = app.seed_admin().await;
    let podcast_id = app
        .seed_podcast("Main Show", "https://feed.test/main")
        .await;
    // Newest first: "Episode 0000" is the most recent, "Episode 0011" the oldest.
    let episode_ids = app.seed_episodes(podcast_id, EPISODES).await;
    // Force a deterministic listen-state partition for the chips: first 4 Finished,
    // next 3 Played (in progress); the rest keep no status row (== Unplayed).
    app.set_playback_status(&episode_ids[..FINISHED], PlaybackStatus::Finished)
        .await;
    app.set_playback_status(
        &episode_ids[FINISHED..FINISHED + IN_PROGRESS],
        PlaybackStatus::Played,
    )
    .await;

    let Some((_driver_guard, driver)) = browser_session().await else {
        return;
    };

    run_session(driver, "list controls journey", async |driver| {
        login_via_ui(&driver, &app.base_url, &admin.username, &admin.password).await;

        driver.goto(format!("{}/latest", app.base_url)).await?;

        // Baseline: all episodes on one page, newest (Episode 0000) on top.
        let initial = wait_for_count_eq(&driver, ROW, EPISODES, Duration::from_secs(10)).await;
        assert_eq!(
            initial,
            EPISODES,
            "expected {EPISODES} rows on first load; body:\n{}",
            body_text(&driver).await
        );
        assert_eq!(
            first_text(&driver, ROW).await,
            "Episode 0000",
            "default sort should be published-desc (newest first)"
        );

        // ── SORT ────────────────────────────────────────────────────────────
        // Open the sort dropdown and click the "Published" field: it's already the
        // active field, so this flips the direction to ascending (oldest first).
        open_dropdown(&driver, "Sort").await?;
        let published = driver
            .query(By::XPath(
                "//button[.//span[normalize-space()='Published']]",
            ))
            .first()
            .await?;
        click_el(&driver, &published).await?;
        // Oldest first now: the last-seeded episode is the top row.
        let oldest = format!("Episode {:04}", EPISODES - 1);
        for _ in 0..40 {
            if first_text(&driver, ROW).await == oldest {
                break;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        assert_eq!(
            first_text(&driver, ROW).await,
            oldest,
            "flipping the Published field to ascending should put the oldest row on top; body:\n{}",
            body_text(&driver).await
        );

        // Flip it back to descending so the search/chip steps start from the default.
        open_dropdown(&driver, "Sort").await?;
        let published = driver
            .query(By::XPath(
                "//button[.//span[normalize-space()='Published']]",
            ))
            .first()
            .await?;
        click_el(&driver, &published).await?;
        for _ in 0..40 {
            if first_text(&driver, ROW).await == "Episode 0000" {
                break;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        assert_eq!(
            first_text(&driver, ROW).await,
            "Episode 0000",
            "sort should flip back to desc"
        );

        // ── SEARCH ──────────────────────────────────────────────────────────
        // Reveal the search bar, type a title-unique substring → exactly one row.
        click(&driver, "button[aria-label='Search']").await?;
        assert!(
            wait_for_css(
                &driver,
                "input[placeholder='Search...']",
                Duration::from_secs(5)
            )
            .await,
            "search bar did not appear after tapping the search toggle"
        );
        let search = driver
            .query(By::Css("input[placeholder='Search...']"))
            .first()
            .await?;
        // "0001" appears in "Episode 0001" only (max title is 0011 → no "00010").
        search.send_keys("0001").await?;
        let narrowed = wait_for_count_eq(&driver, ROW, 1, Duration::from_secs(10)).await;
        assert_eq!(
            narrowed,
            1,
            "search '0001' should narrow to a single episode; body:\n{}",
            body_text(&driver).await
        );
        assert_eq!(
            first_text(&driver, ROW).await,
            "Episode 0001",
            "the one match is Episode 0001"
        );

        // The in-field clear (X) button erases the query but keeps the bar OPEN.
        click(&driver, "button[aria-label='Clear search']").await?;
        let cleared = wait_for_count_eq(&driver, ROW, EPISODES, Duration::from_secs(10)).await;
        assert_eq!(
            cleared,
            EPISODES,
            "the clear button should restore all {EPISODES} rows; body:\n{}",
            body_text(&driver).await
        );
        assert!(
            wait_for_css(
                &driver,
                "input[placeholder='Search...']",
                Duration::from_secs(2)
            )
            .await,
            "the clear button must NOT close the search bar"
        );

        // Closing the bar via the toggle reverts the query (search isn't persisted):
        // type again, close, reopen → the full list is back, not the single match.
        let search = driver
            .query(By::Css("input[placeholder='Search...']"))
            .first()
            .await?;
        search.send_keys("0002").await?;
        assert_eq!(
            wait_for_count_eq(&driver, ROW, 1, Duration::from_secs(10)).await,
            1,
            "search '0002' should narrow to one row"
        );
        click(&driver, "button[aria-label='Search']").await?; // close → revert
        click(&driver, "button[aria-label='Search']").await?; // reopen
        let reverted = wait_for_count_eq(&driver, ROW, EPISODES, Duration::from_secs(10)).await;
        assert_eq!(
            reverted,
            EPISODES,
            "closing the search bar should revert the query (full set on reopen); body:\n{}",
            body_text(&driver).await
        );

        // ── CHIPS ───────────────────────────────────────────────────────────
        // Finished → only the finished subset.
        toggle_chip(&driver, "Finished").await?;
        let finished = wait_for_count_eq(&driver, ROW, FINISHED, Duration::from_secs(10)).await;
        assert_eq!(
            finished,
            FINISHED,
            "the Finished chip should show the {FINISHED} finished episodes; body:\n{}",
            body_text(&driver).await
        );

        // Uncheck Finished → full set.
        toggle_chip(&driver, "Finished").await?;
        let cleared = wait_for_count_eq(&driver, ROW, EPISODES, Duration::from_secs(10)).await;
        assert_eq!(
            cleared, EPISODES,
            "unchecking Finished should restore the full set"
        );

        // In Progress → only the started-but-not-finished subset.
        toggle_chip(&driver, "In Progress").await?;
        let in_progress =
            wait_for_count_eq(&driver, ROW, IN_PROGRESS, Duration::from_secs(10)).await;
        assert_eq!(
            in_progress,
            IN_PROGRESS,
            "the In Progress chip should show the {IN_PROGRESS} in-progress episodes; body:\n{}",
            body_text(&driver).await
        );

        // Uncheck In Progress → full set.
        toggle_chip(&driver, "In Progress").await?;
        let cleared = wait_for_count_eq(&driver, ROW, EPISODES, Duration::from_secs(10)).await;
        assert_eq!(
            cleared, EPISODES,
            "unchecking In Progress should restore the full set"
        );

        // Unplayed → only the never-started remainder.
        toggle_chip(&driver, "Unplayed").await?;
        let unplayed = wait_for_count_eq(&driver, ROW, UNPLAYED, Duration::from_secs(10)).await;
        assert_eq!(
            unplayed,
            UNPLAYED,
            "the Unplayed chip should show the {UNPLAYED} never-started episodes; body:\n{}",
            body_text(&driver).await
        );

        Ok::<_, WebDriverError>(())
    })
    .await;
}
