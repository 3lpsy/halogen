//! Journey A — feed → play → now-playing → history.
//!
//! The end-to-end playback journey through the real UI. The feed is ingested
//! server-side (seed + poll, not the UI subscribe form — that async round-trip
//! doesn't reliably land in the test window); the browser then plays the newest
//! episode from /latest (the mini player reaches the Playing state), expands to
//! the full-screen now-playing overlay and drives its controls (seek slider,
//! speed control, skip-forward), closes the player (the mini player disappears),
//! then confirms the episode shows up on /history.
//!
//! Only upstream RSS is faked (a `wiremock` feed); the download service is the
//! mock-download copy so play sources real bytes. `#[ignore]` by default; run
//! via `just test-e2e`.

use halogen_e2e::{
    body_text, browser_session, click, ingest_feed, login_via_ui, require_dist, run_session,
    wait_for_css, wait_for_text,
};
use halogen_integ::support;
use std::time::Duration;
use thirtyfour::error::WebDriverError;
use thirtyfour::prelude::*;

#[tokio::test]
#[ignore = "needs Chrome + chromedriver + a built dist/ (run via `just test-e2e`)"]
async fn play_now_playing_journey() {
    if !require_dist() {
        return;
    }

    // Mock download ON so play sources real bytes (the server "fetches" the
    // nasa-test-clip fixture into the test media_root, then the client streams
    // its server copy).
    let app = support::spawn_with(support::SpawnOptions {
        use_mock_download: true,
        ..Default::default()
    })
    .await;
    let admin = app.seed_admin().await;

    // Ingest the feed deterministically via a server-side poll — the same proven
    // path the library/download journeys use. The UI subscribe form is an async
    // dispatch→worker→pull round-trip that doesn't reliably land within the test
    // window, so we don't drive it here.
    let _feed = ingest_feed(
        &app,
        &admin.token,
        "Software Engineering Daily",
        "sed_podcast.xml",
    )
    .await;

    // The poll only ingests metadata. The UI's local-first play gating keeps the
    // play badge DISABLED until the server actually holds the file, so stage the
    // newest episode (the first row /latest shows) on the server — mock download
    // copies the fixture clip into media_root. Done before the browser opens so
    // the client's first sync pull already sees `Downloaded` and the badge is
    // enabled on first render.
    let newest = app.newest_episode_id().await;
    app.download_on_server(newest).await;

    let Some((_driver_guard, driver)) = browser_session().await else {
        return;
    };

    run_session(driver, "play / now-playing journey", async |driver| {
        login_via_ui(&driver, &app.base_url, &admin.username, &admin.password).await;

        // ── Play the newest episode from /latest (the client syncs it down) ──
        driver.goto(format!("{}/latest", app.base_url)).await?;
        assert!(
            wait_for_css(&driver, "#episode-scroll h2", Duration::from_secs(10)).await,
            "no episodes rendered on Latest; body:\n{}",
            body_text(&driver).await
        );
        let title = driver
            .query(By::Css("#episode-scroll h2"))
            .first()
            .await?
            .text()
            .await?;
        assert!(!title.is_empty(), "first episode title was empty");

        // The enabled play badge (server has the file via mock download).
        let play = driver
            .query(By::Css(".badge.badge-outline.cursor-pointer"))
            .first()
            .await?;
        play.click().await?;

        // Mini player appears and reaches the Playing state (toggle → Pause).
        assert!(
            wait_for_css(&driver, "#mini-player", Duration::from_secs(10)).await,
            "mini player never appeared after play; body:\n{}",
            body_text(&driver).await
        );
        assert!(
            wait_for_css(
                &driver,
                "#mini-player button[aria-label='Pause']",
                Duration::from_secs(10),
            )
            .await,
            "playback never reached Playing; body:\n{}",
            body_text(&driver).await
        );

        // ── Expand to the now-playing overlay and drive its controls ────────
        // Tapping the metadata button (artwork/title) opens the full-screen view.
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
        // Seek slider + speed control are both present.
        assert!(
            wait_for_css(&driver, "input[type='range']", Duration::from_secs(5)).await,
            "seek slider missing on now-playing"
        );
        assert!(
            wait_for_css(
                &driver,
                "[aria-label='Playback speed']",
                Duration::from_secs(5)
            )
            .await,
            "speed control missing on now-playing"
        );

        // Skip forward (the "+30s" transport button — default skip interval).
        driver
            .query(By::XPath("//button[starts-with(normalize-space(),'+')]"))
            .first()
            .await?
            .click()
            .await?;

        // Set speed to 2x via the speed icon dropdown (no longer a native <select>):
        // open the trigger, then click the 2x rate (f32 `2.0` renders "2x").
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

        // There is no "Stop" control in the now-playing overlay. Collapse the
        // overlay via its Collapse button (the ChevronDown), then stop playback
        // via the mini player's Close button (which calls `stop()`) — the mini
        // player then disappears. Target by aria-label, not `.fixed.inset-0 button`:
        // the app-layout root is also `fixed inset-0` and now holds the navbar's
        // "Go Offline" toggle, which that bare selector would hit instead. Use the
        // overlay-tolerant `click` helper: a short clip can reach its end
        // mid-interaction, and the ended overlay can sit over the mini player,
        // intercepting a geometric click — the helper falls back to a JS click.
        click(&driver, "button[aria-label='Collapse player']")
            .await
            .ok();
        click(&driver, "#mini-player button[aria-label='Close player']").await?;
        for _ in 0..40 {
            if driver.find(By::Css("#mini-player")).await.is_err() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        assert!(
            driver.find(By::Css("#mini-player")).await.is_err(),
            "mini player should be gone after closing the player; body:\n{}",
            body_text(&driver).await
        );

        // ── History reflects the played episode ─────────────────────────────
        driver.goto(format!("{}/history", app.base_url)).await?;
        assert!(
            wait_for_text(&driver, &title, Duration::from_secs(10)).await,
            "played episode '{title}' did not appear on History; body:\n{}",
            body_text(&driver).await
        );

        Ok::<_, WebDriverError>(())
    })
    .await;
}
