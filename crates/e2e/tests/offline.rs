//! Offline journey — once a list has been cached online, the app keeps rendering
//! it after the server becomes unreachable (offline-first), and the per-episode
//! play control disables because there's nothing to stream.
//!
//! We never tear down the server (the harness owns its lifetime); instead we point
//! the *client* at a dead address by rewriting the active user's stored config
//! (`patch_active_config`) and reloading. The worker then fails its pull and flips
//! `sync_status` to Offline, while the episode list still resolves from the local
//! store.
//!
//! Covers two things nothing else did: offline rendering of a general list page,
//! and the offline-disabled state of an episode's results/actions.
//!
//! `#[ignore]` by default; run via `just test-e2e`.

use halogen_e2e::{
    body_text, browser_session, login_via_ui, patch_active_config, require_dist, run_session,
    wait_for_count, wait_for_css,
};
use halogen_integ::support;
use std::time::Duration;
use thirtyfour::error::WebDriverError;

const EPISODES: usize = 8;
const ROW: &str = "#episode-scroll h2";

/// While online + not downloaded, the play badge is interactive.
const PLAY_ENABLED: &str = ".badge.badge-outline.cursor-pointer";
/// Offline + not downloaded → the play badge is dimmed + disabled.
const PLAY_DISABLED: &str = ".badge.badge-outline.opacity-40";

#[tokio::test]
#[ignore = "needs Chrome + chromedriver + a built dist/ (run via `just test-e2e`)"]
async fn cached_list_renders_offline_with_play_disabled() {
    if !require_dist() {
        return;
    }

    let app = support::spawn().await;
    let admin = app.seed_admin().await;
    let podcast_id = app
        .seed_podcast("Main Show", "https://feed.test/main")
        .await;
    let episode_ids = app.seed_episodes(podcast_id, EPISODES).await;
    // Play is gated on the SERVER holding the file (local-first: play either
    // downloads-then-plays or streams — both need the server copy). Mark the
    // seeds server-downloaded so the play badge is enabled while online.
    app.set_download_status(&episode_ids, halogen_wire::DownloadStatus::Downloaded)
        .await;

    let Some((_driver_guard, driver)) = browser_session().await else {
        return;
    };

    run_session(driver, "offline journey", async |driver| {
        login_via_ui(&driver, &app.base_url, &admin.username, &admin.password).await;

        // Online: the list loads and caches; the play control is enabled (proving
        // we reached the Online state before we cut the connection).
        driver.goto(format!("{}/latest", app.base_url)).await?;
        let online = wait_for_count(&driver, ROW, EPISODES, Duration::from_secs(10)).await;
        assert!(
            online >= EPISODES,
            "expected {EPISODES} episodes online; body:\n{}",
            body_text(&driver).await
        );
        assert!(
            wait_for_css(&driver, PLAY_ENABLED, Duration::from_secs(10)).await,
            "play control should be enabled while online; body:\n{}",
            body_text(&driver).await
        );

        // Cut the client off: repoint the configured server at a dead local port.
        // (Token + setup flag stay, so the app remains authenticated — only the
        // network is gone.)
        patch_active_config(&driver, "c.server_url = 'http://127.0.0.1:1';", Vec::new()).await?;
        driver.refresh().await?;
        driver.goto(format!("{}/latest", app.base_url)).await?;

        // Offline-first: the cached list still renders without the server.
        let offline = wait_for_count(&driver, ROW, EPISODES, Duration::from_secs(10)).await;
        assert!(
            offline >= EPISODES,
            "cached episodes should still render offline (got {offline}); body:\n{}",
            body_text(&driver).await
        );

        // And the worker's failed pull flips to Offline, disabling play (nothing to
        // stream without the server and no device download).
        assert!(
            wait_for_css(&driver, PLAY_DISABLED, Duration::from_secs(10)).await,
            "play control should be disabled offline; body:\n{}",
            body_text(&driver).await
        );

        Ok::<_, WebDriverError>(())
    })
    .await;
}

/// An unreachable server is the **Offline** condition, not a page error: the
/// cached pool is all we have, exactly as with the manual "Go Offline" toggle,
/// and the offline indicator already carries that state.
///
/// The list used to report it as an error and render the raw `reqwest`/`ApiError`
/// wording — "Error loading episodes: transport error: error sending request" —
/// in a banner above rows that were resolving from cache perfectly well. The
/// error is surfaced only when the cached pool `is_empty()`, sampled when the
/// fetch resolves; a connection-refused fails so much faster than the local-store
/// read that the pool still looks empty, so the banner latched and then stayed
/// there once the rows arrived.
///
/// Separate from [`cached_list_renders_offline_with_play_disabled`] on purpose:
/// that test's play-badge assertion is currently failing on master (the badge no
/// longer disables offline), which would mask this one entirely.
#[tokio::test]
#[ignore = "needs Chrome + chromedriver + a built dist/ (run via `just test-e2e`)"]
async fn offline_list_does_not_surface_raw_transport_error() {
    if !require_dist() {
        return;
    }

    let app = support::spawn().await;
    let admin = app.seed_admin().await;
    let podcast_id = app
        .seed_podcast("Main Show", "https://feed.test/main")
        .await;
    app.seed_episodes(podcast_id, EPISODES).await;

    let Some((_driver_guard, driver)) = browser_session().await else {
        return;
    };

    run_session(driver, "offline error wording", async |driver| {
        login_via_ui(&driver, &app.base_url, &admin.username, &admin.password).await;

        // Online first, so the pool is cached before we cut the connection.
        driver.goto(format!("{}/latest", app.base_url)).await?;
        let online = wait_for_count(&driver, ROW, EPISODES, Duration::from_secs(10)).await;
        assert!(online >= EPISODES, "expected {EPISODES} episodes online");

        // Same cut as the journey above: repoint the client at a dead port, which
        // is a real connection-refused — the shape of a dropped VPN/tunnel.
        patch_active_config(&driver, "c.server_url = 'http://127.0.0.1:1';", Vec::new()).await?;
        driver.refresh().await?;
        driver.goto(format!("{}/latest", app.base_url)).await?;

        // The cache still renders...
        let offline = wait_for_count(&driver, ROW, EPISODES, Duration::from_secs(10)).await;
        assert!(
            offline >= EPISODES,
            "cached episodes should still render offline (got {offline}); body:\n{}",
            body_text(&driver).await
        );
        // ...and no internal error wording rides along with it. Give the failed
        // fetch a moment to land first, or this asserts against a page that hasn't
        // tried the network yet.
        tokio::time::sleep(Duration::from_secs(2)).await;
        let body = body_text(&driver).await;
        for leak in ["transport error", "error sending request", "ApiError"] {
            assert!(
                !body.contains(leak),
                "offline must not surface raw transport wording ({leak:?}); body:\n{body}"
            );
        }

        Ok::<_, WebDriverError>(())
    })
    .await;
}
