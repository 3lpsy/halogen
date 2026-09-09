//! Open an episode, download and play it, follow its podcast link, then confirm podcast deletion and the redirect to a
//! list without that card. Ignored by default; run with `just test-e2e`.

use halogen_e2e::{
    body_text, browser_session, click, click_button_text, login_via_ui, require_dist, run_session,
    wait_for_count, wait_for_css, wait_for_text, wait_for_url,
};
use halogen_integ::support;
use std::time::Duration;
use thirtyfour::error::WebDriverError;
use thirtyfour::prelude::*;

const EPISODES: usize = 4;
const ROW: &str = "#episode-scroll h2";

#[tokio::test]
#[ignore = "needs Chrome + chromedriver + a built dist/ (run via `just test-e2e`)"]
async fn episode_detail_delete_journey() {
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
    app.seed_episodes(podcast_id, EPISODES).await;

    let Some((_driver_guard, driver)) = browser_session().await else {
        return;
    };

    run_session(driver, "episode detail / delete journey", async |driver| {
        login_via_ui(&driver, &app.base_url, &admin.username, &admin.password).await;

        // ── /latest: capture the newest row's title, tap it open ────────────
        driver.goto(format!("{}/latest", app.base_url)).await?;
        let n = wait_for_count(&driver, ROW, EPISODES, Duration::from_secs(10)).await;
        assert!(
            n >= EPISODES,
            "expected episodes on Latest; body:\n{}",
            body_text(&driver).await
        );
        // Newest first → "Episode 0000".
        let title = driver.query(By::Css(ROW)).first().await?.text().await?;
        assert_eq!(
            title, "Episode 0000",
            "newest seeded episode should be on top"
        );

        // Click the row title → episode detail. (Whole-card tap no longer
        // navigates; only the title text and the chevron do.)
        click(&driver, ROW).await?;
        assert!(
            wait_for_url(&driver, "/episodes/", Duration::from_secs(10)).await,
            "clicking the row title did not navigate to the episode detail; body:\n{}",
            body_text(&driver).await
        );
        // h1 on the detail equals the row title.
        assert!(
            wait_for_css(&driver, "h1", Duration::from_secs(10)).await,
            "episode detail did not render; body:\n{}",
            body_text(&driver).await
        );
        let h1 = driver.query(By::Css("h1")).first().await?.text().await?;
        assert_eq!(h1, title, "episode detail h1 should equal the tapped title");

        // ── Download to device via the back-row actions menu ────────────────
        // The download/stream controls that used to sit under Play now live in the
        // kebab (⋯) on the back-button row. Open it and pick "Download to device".
        click(&driver, "button[aria-label='Episode actions']").await?;
        // The menu carries the (relocated) server download actions too.
        assert!(
            wait_for_text(&driver, "on server", Duration::from_secs(5)).await,
            "the actions menu should carry the server download options; body:\n{}",
            body_text(&driver).await
        );
        click_button_text(&driver, "Download to device").await?;
        // The menu closes on select; the mock device download completes, which makes
        // the local copy playable → the Play button un-disables.
        assert!(
            wait_for_css(
                &driver,
                "button.btn-primary:not([disabled])",
                Duration::from_secs(10),
            )
            .await,
            "device download never enabled Play on the detail page; body:\n{}",
            body_text(&driver).await
        );

        // ── Play → the mini player appears ──────────────────────────────────
        // Target the ENABLED primary (the just-un-disabled Play button) via the
        // overlay/stale-tolerant helper: `.first()` could otherwise grab a disabled
        // primary, and a device-download re-render can stale a raw handle.
        click(&driver, "button.btn-primary:not([disabled])").await?;
        assert!(
            wait_for_css(&driver, "#mini-player", Duration::from_secs(10)).await,
            "mini player never appeared after Play on detail; body:\n{}",
            body_text(&driver).await
        );

        // ── Follow the podcast-name link → podcast detail ───────────────────
        driver
            .query(By::Css(&format!("a[href$='/podcasts/{podcast_id}']")))
            .first()
            .await?
            .click()
            .await?;
        assert!(
            wait_for_url(
                &driver,
                &format!("/podcasts/{podcast_id}"),
                Duration::from_secs(10),
            )
            .await,
            "podcast link did not navigate to the podcast detail; body:\n{}",
            body_text(&driver).await
        );
        assert!(
            wait_for_text(&driver, "Main Show", Duration::from_secs(10)).await,
            "podcast detail header missing; body:\n{}",
            body_text(&driver).await
        );

        // ── Delete the podcast → kebab menu → confirm modal → redirect ──────
        // The header actions moved into a kebab (⋯) on the back-button row; open
        // it, then pick "Delete" to open the confirm modal.
        click(&driver, "button[aria-label='Podcast actions']").await?;
        click_button_text(&driver, "Delete").await?;
        assert!(
            wait_for_text(&driver, "Delete podcast?", Duration::from_secs(10)).await,
            "delete-confirm modal did not open; body:\n{}",
            body_text(&driver).await
        );
        // Confirm via the modal-action Delete button (scoped so it's not the
        // header button).
        click(&driver, ".modal-action button.btn-error").await?;

        // Redirects to /podcasts and the card is gone.
        assert!(
            wait_for_url(&driver, "/podcasts", Duration::from_secs(10)).await,
            "delete did not redirect to /podcasts; body:\n{}",
            body_text(&driver).await
        );
        for _ in 0..40 {
            if !body_text(&driver).await.contains("Main Show") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        assert!(
            !body_text(&driver).await.contains("Main Show"),
            "deleted podcast card should be gone from /podcasts; body:\n{}",
            body_text(&driver).await
        );

        Ok::<_, WebDriverError>(())
    })
    .await;
}
