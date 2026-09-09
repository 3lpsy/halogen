//! Add an episode through its kebab menu, synthesize a right swipe past 80px to remove it from Queue, and verify
//! persistence after reload. Left swipe downloads instead. Ignored by default; run with `just test-e2e`.

use halogen_e2e::{
    body_text, browser_session, click_button_text, count, login_via_ui, require_dist, run_session,
    wait_for_count, wait_for_text,
};
use halogen_integ::support;
use std::time::Duration;
use thirtyfour::error::WebDriverError;
use thirtyfour::prelude::*;

const EPISODES: usize = 5;
const ROW: &str = "#episode-scroll h2";
/// The draggable swipe card inside each episode row.
const CARD: &str = "#episode-scroll div.card";

/// Synthesize a horizontal swipe of `dx` pixels on the `index`-th element
/// matching `selector` by dispatching real PointerEvents (down → moves →
/// up). Mirrors a finger drag: the item reads `client_coordinates().x`, so the
/// events carry monotonically advancing `clientX` values.
async fn swipe(driver: &WebDriver, selector: &str, index: usize, dx: f64) -> WebDriverResult<()> {
    let script = r#"
        const sel = arguments[0];
        const idx = arguments[1];
        const dx = arguments[2];
        const el = document.querySelectorAll(sel)[idx];
        if (!el) { return 'no-element'; }
        const r = el.getBoundingClientRect();
        const y = r.top + r.height / 2;
        const x0 = r.left + r.width / 2;
        // `buttons` mirrors a REAL pointer drag: the primary button/contact is
        // held (bit 0 = 1) through pointerdown + every pointermove, and released
        // (0) at pointerup. Omitting it (defaulting to 0) misrepresents the
        // gesture — a real touch/mouse drag never reports "no button held"
        // mid-drag, and the app relies on that to tell a held drag from a
        // released one (a mouse-up outside the row must end the swipe).
        const fire = (type, x, buttons) => {
            el.dispatchEvent(new PointerEvent(type, {
                bubbles: true, cancelable: true, pointerId: 1, pointerType: 'touch',
                button: 0, buttons,
                clientX: x, clientY: y,
            }));
        };
        fire('pointerdown', x0, 1);
        // Several moves so the offset ramps past the 80px threshold smoothly.
        for (let i = 1; i <= 5; i++) { fire('pointermove', x0 + (dx * i) / 5, 1); }
        fire('pointerup', x0 + dx, 0);
        return 'ok';
    "#;
    let ret = driver
        .execute(
            script,
            vec![
                serde_json::json!(selector),
                serde_json::json!(index),
                serde_json::json!(dx),
            ],
        )
        .await?;
    assert_eq!(
        ret.json().as_str(),
        Some("ok"),
        "swipe could not find element {selector}[{index}]"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Chrome + chromedriver + a built dist/ (run via `just test-e2e`)"]
async fn queue_lifecycle_journey() {
    if !require_dist() {
        return;
    }

    let app = support::spawn().await;
    let admin = app.seed_admin().await;
    let podcast_id = app
        .seed_podcast("Main Show", "https://feed.test/main")
        .await;
    app.seed_episodes(podcast_id, EPISODES).await;
    // The queue resolves to the `is_default` playlist (`GET /playlists/default`),
    // so seed one marked default. It starts empty.
    app.seed_playlist("Queue", true).await;

    let Some((_driver_guard, driver)) = browser_session().await else {
        return;
    };

    run_session(driver, "queue lifecycle journey", async |driver| {
        login_via_ui(&driver, &app.base_url, &admin.username, &admin.password).await;

        // ── Add the first /latest episode to the queue via its kebab ────────
        driver.goto(format!("{}/latest", app.base_url)).await?;
        let n = wait_for_count(&driver, ROW, EPISODES, Duration::from_secs(10)).await;
        assert!(
            n >= EPISODES,
            "expected episodes on Latest; body:\n{}",
            body_text(&driver).await
        );
        let title = driver.query(By::Css(ROW)).first().await?.text().await?;
        assert!(!title.is_empty(), "first episode title was empty");

        driver
            .query(By::Css("button[aria-label='Episode actions']"))
            .first()
            .await?
            .click()
            .await?;
        assert!(
            wait_for_text(&driver, "Add to queue", Duration::from_secs(5)).await,
            "kebab menu did not open with 'Add to queue'; body:\n{}",
            body_text(&driver).await
        );
        click_button_text(&driver, "Add to queue").await?;
        tokio::time::sleep(Duration::from_secs(1)).await;

        // ── /queue shows the queued episode ─────────────────────────────────
        driver.goto(format!("{}/queue", app.base_url)).await?;
        assert!(
            wait_for_text(&driver, &title, Duration::from_secs(10)).await,
            "queued episode '{title}' did not appear on /queue; body:\n{}",
            body_text(&driver).await
        );
        let before = wait_for_count(&driver, ROW, 1, Duration::from_secs(10)).await;
        assert_eq!(before, 1, "queue should hold exactly one episode");

        // ── Swipe RIGHT → RemoveFromQueue (the Queue's swipe-right action) ───
        swipe(&driver, CARD, 0, 120.0).await?;
        // Count drops to zero.
        for _ in 0..40 {
            if count(&driver, ROW).await == 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        assert_eq!(
            count(&driver, ROW).await,
            0,
            "RemoveFromQueue swipe should empty the queue; body:\n{}",
            body_text(&driver).await
        );

        // ── Removal survives a reload (persisted, not just in-memory) ───────
        driver.refresh().await?;
        driver.goto(format!("{}/queue", app.base_url)).await?;
        // A single fixed-delay sample would pass even if the store re-hydration is slow (the row is briefly
        // absent BEFORE hydration, so an early sample sees 0 whether or not removal persisted). Instead assert
        // the row stays absent across the whole hydration window: if the removal did NOT persist, re-hydration
        // re-adds it within this span and the poll trips.
        for _ in 0..15 {
            assert_eq!(
                count(&driver, ROW).await,
                0,
                "queue removal did not persist across reload (a row reappeared \
                 after hydration); body:\n{}",
                body_text(&driver).await
            );
            tokio::time::sleep(Duration::from_millis(200)).await;
        }

        Ok::<_, WebDriverError>(())
    })
    .await;
}
