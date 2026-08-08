//! Service-worker offline lifecycle — a TRUE (network-severed) cold boot and the
//! SW's cache-activation behavior. Closes the gaps flagged as NM21 (real offline
//! boot) and NM25 (SW activation / `/healthz` bypass).
//!
//! The existing `offline.rs` only repoints the client's `server_url` at a dead
//! port ("warm" offline: the app is already loaded, the API is unreachable). It
//! never exercises a COLD boot with the network actually gone — where the app
//! shell, wasm, and assets must come from the service-worker cache. Here we sever
//! the network at the browser via CDP `Network.emulateNetworkConditions`
//! (`offline:true`) and reload, so only the SW can answer.
//!
//! `#[ignore]` by default; run via `just test-e2e`.

use halogen_e2e::{
    body_text, browser_session, login_via_ui, require_dist, run_session, sw_cached_body_len,
    sw_cached_pathnames, wait_for_count, wait_for_css, wait_for_sw_control,
};
use halogen_integ::support;
use serde_json::json;
use std::time::Duration;
use thirtyfour::error::{WebDriverError, WebDriverResult};

const EPISODES: usize = 6;
const ROW: &str = "#episode-scroll h2";

/// Toggle the browser's network at the CDP layer (the lighthouse emulation
/// pattern). `offline=true` severs ALL requests — the real cold-offline case a
/// `server_url` repoint can't reproduce.
async fn set_network_offline(driver: &thirtyfour::WebDriver, offline: bool) -> WebDriverResult<()> {
    let cdp = driver.cdp();
    cdp.send_raw("Network.enable", json!({})).await.ok();
    cdp.send_raw(
        "Network.emulateNetworkConditions",
        json!({
            "offline": offline,
            "latency": 0.0,
            "downloadThroughput": -1.0,
            "uploadThroughput": -1.0
        }),
    )
    .await
    .map(|_| ())
}

#[tokio::test]
#[ignore = "needs Chrome + chromedriver + a built dist/ (run via `just test-e2e`)"]
async fn true_offline_cold_boot_serves_app_shell() {
    if !require_dist() {
        return;
    }

    let app = support::spawn().await;
    let admin = app.seed_admin().await;
    let podcast = app
        .seed_podcast("Main Show", "https://feed.test/main")
        .await;
    let ids = app.seed_episodes(podcast, EPISODES).await;
    app.set_download_status(&ids, halogen_wire::DownloadStatus::Downloaded)
        .await;

    let Some((_driver_guard, driver)) = browser_session().await else {
        return;
    };

    run_session(driver, "true offline cold boot", async |driver| {
        login_via_ui(&driver, &app.base_url, &admin.username, &admin.password).await;

        // Prime the caches: load a list online, then wait until the SW controls
        // the page (before that it caches nothing on `fetch`).
        driver.goto(format!("{}/latest", app.base_url)).await?;
        assert!(
            wait_for_count(&driver, ROW, EPISODES, Duration::from_secs(10)).await >= EPISODES,
            "list never rendered online; body:\n{}",
            body_text(&driver).await
        );
        assert!(
            wait_for_sw_control(&driver, Duration::from_secs(20)).await,
            "service worker never took control"
        );

        // Sever the network entirely, then COLD BOOT (reload). Only the SW cache
        // can answer the navigation + wasm/asset requests now.
        set_network_offline(&driver, true).await?;
        driver.refresh().await?;

        // The app shell must render from cache — authenticated chrome present,
        // NOT a browser "no internet" error page. Generous timeout: an OFFLINE
        // cold boot serves the shell + wasm from the SW cache and boots the wasm
        // with no network, which is slower than a warm online load.
        assert!(
            wait_for_css(&driver, "a[href='/queue']", Duration::from_secs(30)).await,
            "app shell did not boot from the SW cache while offline; body:\n{}",
            body_text(&driver).await
        );
        // A cached list still renders from the local store with no network —
        // this + the booted shell above is the NM21 signal (a real cold boot
        // served entirely from the SW cache). Offline *detection* latency (the
        // navbar flipping to "Offline") is a separate concern with nondeterministic
        // timing under CDP emulation, already covered by the warm `offline.rs`
        // test, so it's deliberately not asserted here.
        driver.goto(format!("{}/latest", app.base_url)).await?;
        assert!(
            wait_for_count(&driver, ROW, 1, Duration::from_secs(15)).await >= 1,
            "cached list did not render on the offline cold boot; body:\n{}",
            body_text(&driver).await
        );

        // Restore the network: the app recovers to Online after a reload.
        set_network_offline(&driver, false).await?;
        driver.refresh().await?;
        assert!(
            wait_for_css(&driver, "a[href='/queue']", Duration::from_secs(15)).await,
            "app did not recover after the network was restored"
        );

        Ok::<_, WebDriverError>(())
    })
    .await;
}

/// The `activate` handler in `sw.js` deletes every cache whose name isn't the
/// current build's fingerprinted `CACHE`, and `/healthz` is bypassed (never
/// cached) so the reachability probe can't answer stale — the guard added in
/// NM17. A full version-BUMP eviction test needs two separate dist builds and is
/// out of scope for a single run; what a single build can prove is asserted here.
#[tokio::test]
#[ignore = "needs Chrome + chromedriver + a built dist/ (run via `just test-e2e`)"]
async fn sw_activation_keeps_one_cache_and_never_caches_healthz() {
    if !require_dist() {
        return;
    }

    let app = support::spawn().await;
    let admin = app.seed_admin().await;
    let podcast = app
        .seed_podcast("Main Show", "https://feed.test/main")
        .await;
    app.seed_episodes(podcast, EPISODES).await;

    let Some((_driver_guard, driver)) = browser_session().await else {
        return;
    };

    run_session(driver, "sw activation", async |driver| {
        login_via_ui(&driver, &app.base_url, &admin.username, &admin.password).await;
        driver.goto(format!("{}/latest", app.base_url)).await?;
        assert!(
            wait_for_sw_control(&driver, Duration::from_secs(20)).await,
            "service worker never took control"
        );

        // Exactly one cache bucket survives activation (the fingerprinted one).
        let names = cache_names(driver).await;
        assert_eq!(
            names.len(),
            1,
            "activate should leave exactly one (fingerprinted) cache; got: {names:?}"
        );

        // The app shell IS cached...
        let shell = sw_cached_body_len(driver, "/").await;
        assert!(shell > 0, "app shell '/' should be cached with a body");

        // ...but the reachability probe is NEVER cached (NM17 bypass): fetch it
        // (populating any cache the SW would use), then assert it's absent.
        driver
            .execute_async(
                "const cb = arguments[arguments.length-1];\
                 fetch('/healthz').catch(()=>{}).finally(()=>cb(null));",
                Vec::new(),
            )
            .await
            .ok();
        let cached = sw_cached_pathnames(driver).await;
        assert!(
            !cached.iter().any(|p| p == "/healthz"),
            "/healthz must never be cached by the SW; cache holds: {cached:?}"
        );

        Ok::<_, WebDriverError>(())
    })
    .await;
}

/// The names of every Cache Storage bucket this origin owns.
async fn cache_names(driver: &thirtyfour::WebDriver) -> Vec<String> {
    let script = r#"
        const cb = arguments[arguments.length - 1];
        (async () => { cb(self.caches ? await caches.keys() : []); })().catch(() => cb([]));
    "#;
    driver
        .execute_async(script, Vec::new())
        .await
        .ok()
        .and_then(|r| serde_json::from_value::<Vec<String>>(r.json().clone()).ok())
        .unwrap_or_default()
}
