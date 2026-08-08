//! Service-worker artwork caching — the PWA must still have its list art after
//! the server goes away.
//!
//! The app renders two artwork resolutions from different URLs (see
//! `ui-appstate::media_url`): `/art` (full-size — player, episode detail) and
//! `/art/small` (~256px thumbnail — the episode/podcast LISTS). `sw.js` routes
//! artwork to a cache-first handler and BYPASSES the rest of `/api/` entirely, so
//! whether a given art URL is recognised by `isArt` decides whether it survives
//! offline at all.
//!
//! This asserts BOTH resolutions land in the cache. Only `/art` used to: the
//! `isArt` regex was anchored (`/art$`), so every `/art/small` thumbnail fell into
//! the `/api/` bypass and was never stored — which meant the pages built entirely
//! out of thumbnails (queue, latest, podcasts) were exactly the ones that lost all
//! their art offline, while the full-size player art stayed put.
//!
//! We assert the CACHE, not rendered `<img>`s: an `<img>` only proves the network
//! served it, which stays true right up until the moment you lose the server.
//!
//! `#[ignore]` by default; run via `just test-e2e`.

use halogen_e2e::{
    body_text, browser_session, login_via_ui, require_dist, run_session, sw_cached_body_len,
    sw_cached_pathnames, sw_cached_paths, wait_for_count, wait_for_sw_control,
};
use halogen_integ::support;
use std::time::Duration;
use thirtyfour::error::WebDriverError;

const EPISODES: usize = 3;
const ROW: &str = "#episode-scroll h2";

#[tokio::test]
#[ignore = "needs Chrome + chromedriver + a built dist/ (run via `just test-e2e`)"]
async fn service_worker_caches_both_art_resolutions() {
    if !require_dist() {
        return;
    }

    let app = support::spawn().await;
    let admin = app.seed_admin().await;
    let podcast_id = app
        .seed_podcast("Main Show", "https://feed.test/main")
        .await;
    // Stage REAL artwork so `/art` serves 200 + image bytes (and `/art/small`
    // generates a thumbnail). Without this the art-less seed 204s, and the SW
    // caches empty responses — the test would then pass on key presence alone
    // while proving nothing about artwork actually surviving offline.
    app.seed_podcast_art(podcast_id).await;
    let episode_ids = app.seed_episodes(podcast_id, EPISODES).await;

    let Some((_driver_guard, driver)) = browser_session().await else {
        return;
    };

    run_session(driver, "pwa art cache", async |driver| {
        login_via_ui(&driver, &app.base_url, &admin.username, &admin.password).await;

        // The SW caches on `fetch` events, which it only sees once it CONTROLS the
        // page — before that it's registered but inert, and any cache assertion
        // would race the registration instead of testing it.
        assert!(
            wait_for_sw_control(&driver, Duration::from_secs(20)).await,
            "service worker never took control of the page"
        );

        // Load a list (fetches `/art/small` per row), then the episode detail
        // (fetches the full-size `/art`), so both resolutions have been requested.
        driver.goto(format!("{}/latest", app.base_url)).await?;
        let rows = wait_for_count(&driver, ROW, EPISODES, Duration::from_secs(15)).await;
        assert!(
            rows >= EPISODES,
            "expected {EPISODES} episodes; body:\n{}",
            body_text(&driver).await
        );
        let id = episode_ids[0];
        driver
            .goto(format!("{}/episodes/{id}", app.base_url))
            .await?;
        // Give the art requests a beat to land in the cache (cache.put resolves
        // after the response is served, so a rendered page isn't proof yet).
        tokio::time::sleep(Duration::from_secs(2)).await;

        let full = format!("/api/v1/episodes/{id}/art");
        let small = format!("/api/v1/episodes/{id}/art/small");
        let cached = sw_cached_paths(&driver, &[&full, &small]).await;
        // Whole cache on failure — otherwise a miss is indistinguishable from the
        // SW never having run at all.
        let all = sw_cached_pathnames(&driver).await;

        // The regression: this is the one that was silently bypassed.
        assert!(
            cached.contains(&small),
            "list thumbnail ({small}) must be cached by the service worker \
             — it is what the queue/latest/podcasts pages render, and without it \
             they lose all art offline.\ncache holds: {all:#?}"
        );
        // Guard the arm that already worked, so a regex fix can't trade one for
        // the other.
        assert!(
            cached.contains(&full),
            "full-size art ({full}) must still be cached.\ncache holds: {all:#?}"
        );

        // Presence isn't enough: an art-less row 204s and STILL caches (a 204 is
        // `ok`), so key-presence alone passes with zero real artwork. Assert the
        // cached bodies are actual image bytes — the podcast art was seeded, and
        // episode art falls back to it, so both resolutions carry non-empty
        // bodies. This is what "art survives offline" actually means.
        let full_len = sw_cached_body_len(&driver, &full).await;
        let small_len = sw_cached_body_len(&driver, &small).await;
        assert!(
            full_len > 0,
            "full-size art was cached but with an EMPTY body ({full_len} bytes) — \
             a 204 was cached instead of real artwork"
        );
        assert!(
            small_len > 0,
            "list thumbnail was cached but with an EMPTY body ({small_len} bytes) — \
             a 204 was cached instead of real artwork"
        );

        Ok::<_, WebDriverError>(())
    })
    .await;
}
