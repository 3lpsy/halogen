//! Full session covers onboarding, search/sort, queue playback, expanded controls, a separate device download, offline
//! playback, reconnect, history, and persisted queue/download state after reload. Ignored by default; run with `just
//! test-e2e`.

use halogen_e2e::{
    body_text, browser_session, click, click_button_text, count, first_text, ingest_feed,
    login_via_ui, patch_active_config, require_dist, run_session, wait_for_count, wait_for_css,
    wait_for_text,
};
use halogen_integ::support;
use std::time::Duration;
use thirtyfour::error::WebDriverError;
use thirtyfour::prelude::*;

const ROW: &str = "#episode-scroll h2";

#[tokio::test]
#[ignore = "needs Chrome + chromedriver + a built dist/ (run via `just test-e2e`)"]
async fn flagship_journey() {
    if !require_dist() {
        return;
    }

    let app = support::spawn_with(support::SpawnOptions {
        use_mock_download: true,
        ..Default::default()
    })
    .await;
    let admin = app.seed_admin().await;
    // Seed the default (`is_default`) playlist that the queue resolves to, empty,
    // so the Add-to-queue step makes the only member.
    app.seed_playlist("Queue", true).await;

    // Ingest the feed deterministically via a server-side poll (the proven path);
    // the UI subscribe form is an async round-trip that doesn't reliably land in
    // the test window.
    let _feed = ingest_feed(
        &app,
        &admin.token,
        "Software Engineering Daily",
        "sed_podcast.xml",
    )
    .await;

    // Stage only FreeBSD on the server so its queue play is enabled. Leave other episodes undownloaded to preserve the
    // later target's Download to device button title.
    let freebsd = app.episode_id_by_title("FreeBSD").await;
    app.download_on_server(freebsd).await;

    let Some((_driver_guard, driver)) = browser_session().await else {
        return;
    };

    run_session(driver, "flagship journey", async |driver| {
        // ── Onboard ─────────────────────────────────────────────────────────
        login_via_ui(&driver, &app.base_url, &admin.username, &admin.password).await;

        // ── /latest: sort by Title, then search narrows (client syncs them) ──
        driver.goto(format!("{}/latest", app.base_url)).await?;
        assert!(
            wait_for_css(&driver, ROW, Duration::from_secs(10)).await,
            "no episodes on Latest; body:\n{}",
            body_text(&driver).await
        );
        halogen_e2e::shot(driver, "02-latest").await;
        let default_top = first_text(&driver, ROW).await;

        // Open the sort dropdown (identified by its trigger's aria-label — both
        // list-bar dropdowns are now plain `dropdown`, so the old positional
        // `:not(.dropdown-end)` selector no longer distinguishes them) and pick Title.
        click(&driver, "div[role='button'][aria-label='Sort']").await?;
        driver
            .query(By::XPath("//button[.//span[normalize-space()='Title']]"))
            .first()
            .await?
            .click()
            .await?;
        // The top row changes when ordering flips from published to title.
        let mut changed = false;
        for _ in 0..40 {
            if first_text(&driver, ROW).await != default_top {
                changed = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        assert!(
            changed,
            "sorting by Title did not reorder the list; body:\n{}",
            body_text(&driver).await
        );

        // Search narrows to a title-unique substring from the feed.
        click(&driver, "button[aria-label='Search']").await?;
        assert!(
            wait_for_css(
                &driver,
                "input[placeholder='Search...']",
                Duration::from_secs(5)
            )
            .await,
            "search bar did not appear"
        );
        driver
            .query(By::Css("input[placeholder='Search...']"))
            .first()
            .await?
            .send_keys("FreeBSD")
            .await?;
        assert!(
            wait_for_text(&driver, "FreeBSD", Duration::from_secs(10)).await,
            "search did not surface the 'FreeBSD' episode; body:\n{}",
            body_text(&driver).await
        );
        let queue_title = first_text(&driver, ROW).await;
        assert!(
            queue_title.contains("FreeBSD"),
            "search should narrow to the FreeBSD episode, got '{queue_title}'"
        );

        // ── Add it to the queue via the kebab ───────────────────────────────
        driver
            .query(By::Css("button[aria-label='Episode actions']"))
            .first()
            .await?
            .click()
            .await?;
        assert!(
            wait_for_text(&driver, "Add to queue", Duration::from_secs(5)).await,
            "kebab did not open; body:\n{}",
            body_text(&driver).await
        );
        click_button_text(&driver, "Add to queue").await?;
        tokio::time::sleep(Duration::from_secs(1)).await;

        // ── /queue: the episode is there; play it ───────────────────────────
        driver.goto(format!("{}/queue", app.base_url)).await?;
        assert!(
            wait_for_text(&driver, &queue_title, Duration::from_secs(10)).await,
            "queued episode missing on /queue; body:\n{}",
            body_text(&driver).await
        );
        click(&driver, ".badge.badge-outline.cursor-pointer").await?;
        assert!(
            wait_for_css(&driver, "#mini-player", Duration::from_secs(10)).await,
            "mini player never appeared from /queue; body:\n{}",
            body_text(&driver).await
        );
        assert!(
            wait_for_css(
                &driver,
                "#mini-player button[aria-label='Pause']",
                Duration::from_secs(10),
            )
            .await,
            "queue playback never reached Playing; body:\n{}",
            body_text(&driver).await
        );

        halogen_e2e::shot(driver, "03-queue").await;

        // ── Expand now-playing: skip + 2x speed ─────────────────────────────
        driver
            .query(By::Css("#mini-player button.flex-1"))
            .first()
            .await?
            .click()
            .await?;
        // The overlay no longer shows a "Now Playing" title; its collapse control
        // (unique to the full-screen player) is the open signal.
        assert!(
            wait_for_css(
                &driver,
                "button[aria-label='Collapse player']",
                Duration::from_secs(10),
            )
            .await,
            "now-playing overlay did not open; body:\n{}",
            body_text(&driver).await
        );
        driver
            .query(By::XPath("//button[starts-with(normalize-space(),'+')]"))
            .first()
            .await?
            .click()
            .await?;
        // Speed is now an icon dropdown (not a native <select>): click the trigger
        // to open the upward menu, then pick the 2x rate (f32 `2.0` renders "2x").
        driver
            .query(By::Css("[aria-label='Playback speed']"))
            .first()
            .await?
            .click()
            .await?;
        driver
            .query(By::XPath(
                "//ul[contains(@class,'dropdown-content')]//button[normalize-space()='2x']",
            ))
            .first()
            .await?
            .click()
            .await?;
        // Collapse the overlay back to the app. Target the collapse control by its
        // aria-label — the app-layout root is also `fixed inset-0` and now holds the
        // navbar's "Go Offline" toggle, so a bare `.fixed.inset-0 button` would hit
        // that instead and silently flip the app offline.
        click(&driver, "button[aria-label='Collapse player']")
            .await
            .ok();

        // Persist a playback record — what History is built from. It's emitted only on pause, a ~10s in-play
        // tick debounce, or an end-of-clip MarkPlayed; none reliably fires in the short window before we go
        // offline, so History came up empty. Pausing sends a SetCursor that syncs while ONLINE (tolerant: a
        // short clip may have already ended → MarkPlayed, which records it too).
        click(&driver, "#mini-player button[aria-label='Pause']")
            .await
            .ok();
        // Verify it synced while online, so the end-of-test History check proves
        // the record SURVIVES the offline/reconnect cycle rather than that it
        // ever existed. This also guarantees the SetCursor flushed before the
        // offline cutover.
        driver.goto(format!("{}/history", app.base_url)).await?;
        assert!(
            wait_for_text(&driver, &queue_title, Duration::from_secs(10)).await,
            "played episode '{queue_title}' missing from History while online; body:\n{}",
            body_text(&driver).await
        );

        halogen_e2e::shot(driver, "04-history").await;

        // ── Device-download a DIFFERENT episode (from /latest) ──────────────
        driver.goto(format!("{}/latest", app.base_url)).await?;
        assert!(
            wait_for_css(&driver, ROW, Duration::from_secs(10)).await,
            "Latest did not re-render; body:\n{}",
            body_text(&driver).await
        );
        let download_title = first_text(&driver, ROW).await;
        // FreeBSD already has a downloaded badge. Require the count to increase so the test waits for this episode's
        // bytes before going offline.
        let downloaded_before =
            count(&driver, "button[title='Downloaded on device — remove']").await;
        driver
            .query(By::Css("button[title='Download to device']"))
            .first()
            .await?
            .click()
            .await?;
        assert!(
            wait_for_count(
                &driver,
                "button[title='Downloaded on device — remove']",
                downloaded_before + 1,
                Duration::from_secs(10),
            )
            .await
                > downloaded_before,
            "device download never completed; body:\n{}",
            body_text(&driver).await
        );

        // ── Go offline (dead port) ──────────────────────────────────────────
        patch_active_config(&driver, "c.server_url = 'http://127.0.0.1:1';", Vec::new()).await?;
        driver.refresh().await?;
        driver.goto(format!("{}/downloads", app.base_url)).await?;

        // The downloaded episode plays with the server dead.
        assert!(
            wait_for_css(&driver, ROW, Duration::from_secs(10)).await,
            "downloaded episode missing on /downloads offline; body:\n{}",
            body_text(&driver).await
        );
        click(&driver, ".badge.badge-outline.cursor-pointer").await?;
        // The short fixture may finish before polling observes Pause. Accept an engaged, error-free offline player even
        // if Playing is too brief to catch.
        assert!(
            wait_for_css(&driver, "#mini-player", Duration::from_secs(10)).await,
            "offline playback did not engage (mini player never appeared); body:\n{}",
            body_text(&driver).await
        );
        wait_for_css(
            &driver,
            "#mini-player button[aria-label='Pause']",
            Duration::from_secs(3),
        )
        .await; // best-effort: caught it mid-play if the clip was long enough
        halogen_e2e::shot(driver, "05-offline-playback").await;
        let player_body = body_text(&driver).await;
        assert!(
            !player_body.contains("Playback error")
                && !player_body.contains("audio error")
                && !player_body.contains("Playback timed out"),
            "offline playback errored instead of playing from the local blob; body:\n{player_body}"
        );
        // The navbar reflects the offline state.
        assert!(
            wait_for_text(&driver, "Offline", Duration::from_secs(10)).await,
            "navbar did not show 'Offline'; body:\n{}",
            body_text(&driver).await
        );

        // ── Reconnect: restore the real server URL + refresh ────────────────
        let real_url = app.base_url.clone();
        patch_active_config(
            &driver,
            "c.server_url = arguments[0];",
            vec![serde_json::json!(real_url)],
        )
        .await?;
        driver.refresh().await?;
        driver.goto(format!("{}/latest", app.base_url)).await?;
        assert!(
            wait_for_text(&driver, "Online", Duration::from_secs(10)).await,
            "navbar did not return to 'Online' after reconnect; body:\n{}",
            body_text(&driver).await
        );

        // /history shows the episode we played from the queue.
        driver.goto(format!("{}/history", app.base_url)).await?;
        assert!(
            wait_for_text(&driver, &queue_title, Duration::from_secs(10)).await,
            "played episode '{queue_title}' missing from History; body:\n{}",
            body_text(&driver).await
        );

        // ── Reload: queue membership + device download persist ──────────────
        driver.refresh().await?;
        driver.goto(format!("{}/queue", app.base_url)).await?;
        assert!(
            wait_for_text(&driver, &queue_title, Duration::from_secs(10)).await,
            "queue membership did not survive reload; body:\n{}",
            body_text(&driver).await
        );
        driver.goto(format!("{}/downloads", app.base_url)).await?;
        assert!(
            wait_for_text(&driver, &download_title, Duration::from_secs(10)).await,
            "device download did not survive reload; body:\n{}",
            body_text(&driver).await
        );

        Ok::<_, WebDriverError>(())
    })
    .await;
}
