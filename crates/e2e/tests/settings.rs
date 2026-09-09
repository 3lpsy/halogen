//! Verify playback preferences persist in the active account's client config across reload and Server settings show the
//! configured URL. Ignored by default; run with `just test-e2e`.

use halogen_e2e::{
    active_config_json, body_text, browser_session, click, click_button_text, login_via_ui,
    require_dist, run_session, wait_for_text,
};
use halogen_integ::support;
use std::time::Duration;
use thirtyfour::Key;
use thirtyfour::error::WebDriverError;
use thirtyfour::prelude::*;

/// Read the `value` attribute of the input whose label is `label` (the spans
/// "Skip forward" / "Skip backward" sit next to their number inputs).
async fn labeled_number_value(driver: &WebDriver, label: &str) -> WebDriverResult<String> {
    let xpath = format!("//div[span[normalize-space()='{label}']]//input[@type='number']");
    let el = driver.query(By::XPath(xpath)).first().await?;
    Ok(el.value().await?.unwrap_or_default())
}

#[tokio::test]
#[ignore = "needs Chrome + chromedriver + a built dist/ (run via `just test-e2e`)"]
async fn settings_persist_playback_prefs() {
    if !require_dist() {
        return;
    }

    let app = support::spawn().await;
    let admin = app.seed_admin().await;

    let Some((_driver_guard, driver)) = browser_session().await else {
        return;
    };

    run_session(driver, "settings journey", async |driver| {
        login_via_ui(&driver, &app.base_url, &admin.username, &admin.password).await;

        // The Server sub-page shows the configured base URL (proves config read).
        driver
            .goto(format!("{}/settings/server", app.base_url))
            .await?;
        assert!(
            wait_for_text(&driver, &app.base_url, Duration::from_secs(10)).await,
            "Server URL not shown on the server settings page; body:\n{}",
            body_text(&driver).await
        );

        // The playback prefs live on their own sub-page now.
        driver
            .goto(format!("{}/settings/playback", app.base_url))
            .await?;
        assert!(
            wait_for_text(&driver, "Skip Intervals", Duration::from_secs(10)).await,
            "playback settings page did not render; body:\n{}",
            body_text(&driver).await
        );

        // Default skip-forward is 30 seconds.
        assert_eq!(
            labeled_number_value(&driver, "Skip forward (seconds)").await?,
            "30",
            "default skip-forward should be 30s"
        );

        // Replace it with 45. The field is a *controlled* input (its value is
        // re-asserted from the signal each render), so a bare `clear()` can snap
        // back to "30" and leave "45" appended as "3045". Select-all then type so
        // each keystroke replaces the whole value instead of appending.
        let skip_fwd = driver
            .query(By::XPath(
                "//div[span[normalize-space()='Skip forward (seconds)']]//input[@type='number']",
            ))
            .first()
            .await?;
        skip_fwd.click().await?;
        skip_fwd.send_keys(Key::Control + "a").await?;
        skip_fwd.send_keys("45").await?;

        // Toggle auto-advance off (default is on).
        let auto = driver
            .query(By::XPath(
                "//div[span[normalize-space()='Automatically play next episode in queue']]//input[@type='checkbox']",
            ))
            .first()
            .await?;
        let was_checked = auto.is_selected().await?;
        assert!(was_checked, "auto-advance should default to on");
        auto.click().await?;

        // Playback source preference: defaults to DownloadOnly (local-first);
        // switch to StreamFallback via the select.
        let pref = driver
            .query(By::XPath(
                // Identify the Playback Source select by a unique option value
                // (robust to the surrounding flex layout) — the rate select has no
                // such option.
                "//select[option[@value='StreamFallback']]",
            ))
            .first()
            .await?;
        assert_eq!(
            pref.value().await?.as_deref(),
            Some("DownloadOnly"),
            "playback preference should default to DownloadOnly"
        );
        driver
            .query(By::XPath(
                "//select/option[@value='StreamFallback']",
            ))
            .first()
            .await?
            .click()
            .await?;

        // The save effect runs on each change; give it a beat to hit localStorage.
        tokio::time::sleep(Duration::from_millis(500)).await;

        // Reload: the form must re-hydrate the persisted values from config.
        driver.refresh().await?;
        assert!(
            wait_for_text(&driver, "Skip Intervals", Duration::from_secs(10)).await,
            "playback settings page did not re-render after reload; body:\n{}",
            body_text(&driver).await
        );
        assert_eq!(
            labeled_number_value(&driver, "Skip forward (seconds)").await?,
            "45",
            "skip-forward should persist across reload; body:\n{}",
            body_text(&driver).await
        );
        let auto_after = driver
            .query(By::XPath(
                "//div[span[normalize-space()='Automatically play next episode in queue']]//input[@type='checkbox']",
            ))
            .first()
            .await?;
        assert!(
            !auto_after.is_selected().await?,
            "auto-advance off should persist across reload"
        );
        let pref_after = driver
            .query(By::XPath(
                // Identify the Playback Source select by a unique option value
                // (robust to the surrounding flex layout) — the rate select has no
                // such option.
                "//select[option[@value='StreamFallback']]",
            ))
            .first()
            .await?;
        assert_eq!(
            pref_after.value().await?.as_deref(),
            Some("StreamFallback"),
            "playback preference should persist across reload"
        );

        // Belt-and-braces: the active user's persisted config carries the value.
        let stored = active_config_json(&driver).await;
        assert!(
            stored.contains("\"skip_forward\":45"),
            "active user's config should hold skip_forward=45; got:\n{stored}"
        );

        Ok::<_, WebDriverError>(())
    })
    .await;
}

/// The admin-only "View config" button opens the reconciled-config page, which
/// renders the grouped sections — and never leaks the secret fields. Logged in
/// as the seeded admin, so the button is visible; the page fetches
/// `GET /api/v1/config` with the admin token.
#[tokio::test]
#[ignore = "needs Chrome + chromedriver + a built dist/ (run via `just test-e2e`)"]
async fn settings_view_config_shows_reconciled_config_without_secrets() {
    if !require_dist() {
        return;
    }

    let app = support::spawn().await;
    let admin = app.seed_admin().await;

    let Some((_driver_guard, driver)) = browser_session().await else {
        return;
    };

    run_session(driver, "view config journey", async |driver| {
        login_via_ui(&driver, &app.base_url, &admin.username, &admin.password).await;

        // The admin server surfaces live on the Server sub-page now.
        driver
            .goto(format!("{}/settings/server", app.base_url))
            .await?;
        assert!(
            wait_for_text(&driver, "View config", Duration::from_secs(10)).await,
            "admin 'View config' button missing on server settings; body:\n{}",
            body_text(&driver).await
        );

        // Open the config page via the button (not a direct URL).
        click_button_text(&driver, "View config").await?;

        // The page title + a couple of section headers prove the grouped, data-
        // loaded view rendered (sections only appear once the fetch resolves Ok).
        assert!(
            wait_for_text(&driver, "Server Configuration", Duration::from_secs(10)).await,
            "config page title did not render; body:\n{}",
            body_text(&driver).await
        );
        for header in ["Database", "Logging", "Other"] {
            assert!(
                wait_for_text(&driver, header, Duration::from_secs(10)).await,
                "config section '{header}' missing; body:\n{}",
                body_text(&driver).await
            );
        }

        // Secrets must never reach the page. The test harness signs JWTs with
        // this secret and the admin logs in with this password — neither may
        // appear in the rendered config.
        let body = body_text(&driver).await;
        assert!(
            !body.contains("test-secret-key-for-jwt-signing"),
            "the JWT/token secret leaked onto the config page; body:\n{body}"
        );
        assert!(
            !body.contains(&admin.password),
            "the admin password leaked onto the config page; body:\n{body}"
        );

        Ok::<_, WebDriverError>(())
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Chrome + chromedriver + a built dist/ (run via `just test-e2e`)"]
async fn settings_delete_local_data_wipes_via_cache_control() {
    if !require_dist() {
        return;
    }

    let app = support::spawn().await;
    let admin = app.seed_admin().await;

    let Some((_driver_guard, driver)) = browser_session().await else {
        return;
    };

    run_session(
        driver,
        "settings → cache-control wipe journey",
        async |driver| {
            login_via_ui(&driver, &app.base_url, &admin.username, &admin.password).await;

            driver.goto(format!("{}/settings", app.base_url)).await?;
            assert!(
                wait_for_text(&driver, "Delete local data", Duration::from_secs(10)).await,
                "settings menu did not render; body:\n{}",
                body_text(&driver).await
            );

            // Settings no longer wipes inline — "Delete local data" is a menu link
            // to the standalone Cache Control page, where the granular and full
            // wipes live.
            click(&driver, "a[href='/cache-control']").await?;
            assert!(
                wait_for_text(&driver, "Local data & cache", Duration::from_secs(10)).await,
                "Delete local data should open Cache Control; body:\n{}",
                body_text(&driver).await
            );

            // "Clear all & sign out" is a two-step inline confirm; confirming clears all
            // browser storage and reloads back to Cache Control with a result message.
            click_button_text(&driver, "Clear all & sign out").await?;
            assert!(
                wait_for_text(&driver, "Are you sure?", Duration::from_secs(5)).await,
                "the action should ask for confirmation first; body:\n{}",
                body_text(&driver).await
            );
            click_button_text(&driver, "Confirm").await?;

            assert!(
                wait_for_text(
                    &driver,
                    "Signed out and cleared saved data",
                    Duration::from_secs(10),
                )
                .await,
                "the wipe should reload Cache Control with its result message; body:\n{}",
                body_text(&driver).await
            );

            // The wipe cleared localStorage, so no set-up active config remains.
            let stored = active_config_json(&driver).await;
            assert!(
                stored.is_empty() || !stored.contains("\"server_setup\":true"),
                "after wipe there must be no set-up active config; got:\n{stored}"
            );

            Ok::<_, WebDriverError>(())
        },
    )
    .await;
}

/// The Settings menu links to the dedicated device-logs viewer ("Device Logs" →
/// /logs/device) — a simple content-render canary that exercises the page
/// outside the playback / config / wipe journeys. (The Theme placeholder it
/// used to assert was removed.)
#[tokio::test]
#[ignore = "needs Chrome + chromedriver + a built dist/ (run via `just test-e2e`)"]
async fn settings_device_logs_link_renders() {
    if !require_dist() {
        return;
    }

    let app = support::spawn().await;
    let admin = app.seed_admin().await;

    let Some((_driver_guard, driver)) = browser_session().await else {
        return;
    };

    run_session(
        driver,
        "settings device-logs link journey",
        async |driver| {
            login_via_ui(&driver, &app.base_url, &admin.username, &admin.password).await;

            driver.goto(format!("{}/settings", app.base_url)).await?;
            assert!(
                wait_for_text(&driver, "Device Logs", Duration::from_secs(10)).await,
                "settings Device Logs section did not render; body:\n{}",
                body_text(&driver).await
            );
            // The Theme placeholder is gone — it must not reappear.
            assert!(
                !body_text(&driver)
                    .await
                    .contains("Theme selection coming soon"),
                "the removed Theme placeholder is still rendering; body:\n{}",
                body_text(&driver).await
            );

            Ok::<_, WebDriverError>(())
        },
    )
    .await;
}
