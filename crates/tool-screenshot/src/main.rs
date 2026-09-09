//! Capture app states through Chrome, using seeded dist/ by default or --base-url with --user/--pass or
//! SHOT_USER/SHOT_PASS. Mobile 412x823 full-page PNGs go under design/screenshots/web/<datetime> and latest; missing
//! selectors log and continue. Run `just web-screenshots-local` with Chrome/chromedriver.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use anyhow::{Context, Result};
use base64::Engine;
use halogen_integ::support::{self, SpawnOptions, TestApp};
use serde_json::json;
use thirtyfour::prelude::*;

// ── CLI ─────────────────────────────────────────────────────────────────────

struct Args {
    base_url: Option<String>,
    user: Option<String>,
    pass: Option<String>,
    out: Option<PathBuf>,
}

fn parse_args() -> Args {
    let mut a = Args {
        base_url: None,
        user: std::env::var("SHOT_USER").ok(),
        pass: std::env::var("SHOT_PASS").ok(),
        out: std::env::var("SHOT_OUT").ok().map(PathBuf::from),
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--base-url" => a.base_url = it.next(),
            "--user" => a.user = it.next(),
            "--pass" => a.pass = it.next(),
            "--out" => a.out = it.next().map(PathBuf::from),
            "-h" | "--help" => {
                println!(
                    "tool-screenshot — capture every UI state of the app\n\n\
                     USAGE: tool-screenshot [--base-url URL --user U --pass P] [--out DIR]\n\
                     \n\
                     Default (no --base-url): spawns a seeded server over dist/ and logs in\n\
                     as the seeded admin. Output: design/screenshots/web/<datetime>/ (+ latest/).\n\
                     Env: SHOT_USER / SHOT_PASS / SHOT_OUT."
                );
                std::process::exit(0);
            }
            other => eprintln!("warning: ignoring unknown arg {other:?}"),
        }
    }
    a
}

// ── Screenshot capture ───────────────────────────────────────────────────────

/// Owns the run directory + the monotonic shot counter (drives the `NN-` prefix
/// so files sort in capture order).
struct Shooter {
    dir: PathBuf,
    n: AtomicUsize,
}

impl Shooter {
    /// Capture the current page as a full-page PNG named `NN-<name>.png`.
    /// Best-effort: logs and returns on failure (a missing state must not abort
    /// the walk). Full-page via CDP `Page.captureScreenshot`, falling back to a
    /// viewport shot if that's unavailable.
    async fn shot(&self, d: &WebDriver, name: &str) {
        let idx = self.n.fetch_add(1, Ordering::SeqCst);
        let file = self.dir.join(format!("{idx:02}-{name}.png"));
        let bytes = match self.capture(d).await {
            Ok(b) => b,
            Err(e) => {
                eprintln!("  ! shot {name}: capture failed ({e})");
                return;
            }
        };
        if let Err(e) = std::fs::write(&file, &bytes) {
            eprintln!("  ! shot {name}: write failed ({e})");
        } else {
            println!("  · {idx:02}-{name}.png");
        }
    }

    async fn capture(&self, d: &WebDriver) -> Result<Vec<u8>> {
        let v = d
            .cdp()
            .send_raw(
                "Page.captureScreenshot",
                json!({ "format": "png", "captureBeyondViewport": true }),
            )
            .await;
        if let Ok(v) = v
            && let Some(b64) = v.get("data").and_then(|x| x.as_str())
            && let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(b64)
            && !bytes.is_empty()
        {
            return Ok(bytes);
        }
        // Fallback: plain viewport screenshot via WebDriver.
        d.screenshot_as_png().await.context("screenshot_as_png")
    }
}

// ── Browser helpers ───────────────────────────────────────────────────────────

async fn sleep(ms: u64) {
    tokio::time::sleep(Duration::from_millis(ms)).await;
}

/// CDP mobile device metrics — a deterministic 412×823 viewport (the lighthouse
/// mobile profile). Best-effort.
async fn apply_viewport(d: &WebDriver) {
    let _ = d
        .cdp()
        .send_raw(
            "Emulation.setDeviceMetricsOverride",
            json!({
                "width": 412, "height": 823, "deviceScaleFactor": 1.75,
                "mobile": true, "screenWidth": 412, "screenHeight": 823
            }),
        )
        .await;
}

/// Navigate + settle (let the SPA boot / route / render).
async fn goto(d: &WebDriver, base: &str, route: &str, settle_ms: u64) {
    let _ = d.goto(format!("{base}{route}")).await;
    sleep(settle_ms).await;
}

/// Click the first match if present (best-effort). Returns whether it clicked.
async fn try_click(d: &WebDriver, css: &str) -> bool {
    if let Ok(el) = d.query(By::Css(css)).first().await {
        let _ = halogen_e2e::click_el(d, &el).await;
        return true;
    }
    false
}

fn dist_dir() -> Option<PathBuf> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../dist")
        .canonicalize()
        .ok()?;
    dir.join("index.html").is_file().then_some(dir)
}

// ── JS gesture synthesizers (transient states) ────────────────────────────────

/// Start a horizontal swipe on the `index`-th `#episode-scroll` card and HOLD it
/// (pointerdown + partial pointermoves, no pointerup) so the row sits revealed
/// for a screenshot. `dx` > 0 reveals the swipe-right action, < 0 the left one.
/// Faithful `buttons:1` touch pointer (a real drag reports the contact held).
async fn swipe_hold(d: &WebDriver, index: usize, dx: f64) {
    let script = r#"
        const idx = arguments[0], dx = arguments[1];
        const el = document.querySelectorAll('#episode-scroll div.card')[idx];
        if (!el) return 'no-el';
        const r = el.getBoundingClientRect();
        const y = r.top + r.height/2, x0 = r.left + r.width/2;
        const fire = (t,x,b) => el.dispatchEvent(new PointerEvent(t,{
            bubbles:true, cancelable:true, pointerId:7, pointerType:'touch',
            button:0, buttons:b, clientX:x, clientY:y }));
        fire('pointerdown', x0, 1);
        for (let i=1;i<=5;i++) fire('pointermove', x0 + dx*i/5, 1);
        return 'ok';
    "#;
    let _ = d.execute(script, vec![json!(index), json!(dx)]).await;
    sleep(350).await;
}

/// Release a held swipe (`pointercancel`) so the row snaps back.
async fn gesture_release(d: &WebDriver) {
    let script = r#"
        const el = document.querySelector('#episode-scroll div.card');
        if (el) el.dispatchEvent(new PointerEvent('pointercancel',{
            bubbles:true, pointerId:7, pointerType:'touch', buttons:0 }));
        const sc = document.querySelector('#episode-scroll');
        if (sc) sc.dispatchEvent(new PointerEvent('pointercancel',{
            bubbles:true, pointerId:8, pointerType:'touch', buttons:0 }));
    "#;
    let _ = d.execute(script, Vec::new()).await;
    sleep(200).await;
}

/// Drag `#episode-scroll` DOWN from the top and HOLD past the pull-to-refresh arm
/// threshold, so the "Release to refresh" overlay shows for a screenshot.
async fn pull_to_refresh_hold(d: &WebDriver) {
    let script = r#"
        const sc = document.querySelector('#episode-scroll');
        if (!sc) return 'no-el';
        sc.scrollTop = 0;
        const r = sc.getBoundingClientRect();
        const x = r.left + r.width/2, y0 = r.top + 12;
        const fire = (t,y,b) => sc.dispatchEvent(new PointerEvent(t,{
            bubbles:true, cancelable:true, pointerId:8, pointerType:'touch',
            button:0, buttons:b, clientX:x, clientY:y }));
        fire('pointerdown', y0, 1);
        for (let i=1;i<=6;i++) fire('pointermove', y0 + 22*i, 1); // ~130px, past the 80px arm
        return 'ok';
    "#;
    let _ = d.execute(script, Vec::new()).await;
    sleep(400).await;
}

// ── Seeding (self-hosted mode) ────────────────────────────────────────────────

/// A rich library so every screen has realistic content: two podcasts with art,
/// episodes, a populated default Queue + a Favorites playlist, listen history,
/// one server-downloaded episode (enables the play badge + server states).
/// Returns the ids the flow seeds on-device audio for.
async fn seed(app: &TestApp, admin_id: i32) -> Vec<i32> {
    let p1 = app
        .seed_podcast("Tech Talk Weekly", "https://feed.test/tech")
        .await;
    let p2 = app
        .seed_podcast("Science Hour", "https://feed.test/science")
        .await;
    let _ = app.seed_podcast_art(p1).await;
    let _ = app.seed_podcast_art(p2).await;
    let e1 = app.seed_episodes(p1, 20).await;
    let _e2 = app.seed_episodes(p2, 20).await;

    let take = |n: usize| e1[..e1.len().min(n)].to_vec();
    let queue = app.seed_playlist("Queue", true).await;
    app.seed_playlist_episodes(queue, &take(10)).await;
    let favorites = app.seed_playlist("Favorites", false).await;
    app.seed_playlist_episodes(favorites, &take(6)).await;
    app.seed_playbacks(admin_id, &take(12)).await;

    // A server-downloaded episode so the play badge is interactive + the episode
    // detail shows a real Play button.
    app.download_on_server(e1[0]).await;
    // Ids we'll stage on-device audio for (populates /downloads + the "remove"
    // badge) once the browser session's account IndexedDB exists.
    take(3)
}

// ── The walk ─────────────────────────────────────────────────────────────────

/// The list scroll container present on Queue/Latest/History/Downloads.
const EP_SCROLL: &str = "#episode-scroll";

/// Capture every reachable UI state, in an order that puts unauthenticated
/// screens before login. Best-effort throughout.
async fn run_flow(
    s: &Shooter,
    d: &WebDriver,
    base: &str,
    user: &str,
    pass: &str,
    on_device: &[i32],
    settle_ms: u64,
) -> Result<()> {
    // ── 1. Unauthenticated / auth flow (before login) ───────────────────────
    println!("auth flow");
    // One consolidated connect form (server URL + credentials, native parity).
    goto(d, base, "/auth/login", settle_ms).await;
    s.shot(d, "login").await;
    // Invalid URL error.
    let _ = halogen_e2e::fill(d, "input[type='url']", "not a url").await;
    try_click(d, "button[type='submit']").await;
    sleep(600).await;
    s.shot(d, "login-url-error").await;
    // Invalid credentials error (valid URL, wrong password).
    let _ = halogen_e2e::fill(d, "input[type='url']", base).await;
    let _ = halogen_e2e::fill(d, "input[type='text']", "nobody").await;
    let _ = halogen_e2e::fill(d, "input[type='password']", "wrongpw").await;
    try_click(d, "button[type='submit']").await;
    sleep(900).await;
    s.shot(d, "login-error").await;

    // ── 2. Sign in ──────────────────────────────────────────────────────────
    println!("login");
    halogen_e2e::login_via_ui(d, base, user, pass).await;
    sleep(settle_ms).await;

    // Stage on-device audio now that the account's IndexedDB exists (populates
    // /downloads + the on-device "remove" badge). Self-hosted only.
    if !on_device.is_empty() {
        let _ = halogen_e2e::idb_seed_audio(d, on_device).await;
    }

    // ── 3. Core lists ───────────────────────────────────────────────────────
    println!("core lists");
    for (route, name) in [
        ("/queue", "queue"),
        ("/latest", "latest"),
        ("/history", "history"),
        ("/downloads", "downloads"),
        ("/podcasts", "podcasts"),
        ("/playlists", "playlists"),
    ] {
        goto(d, base, route, settle_ms).await;
        s.shot(d, name).await;
    }

    // ── 4. List controls / overlays (on Latest) ─────────────────────────────
    println!("list controls");
    goto(d, base, "/latest", settle_ms).await;
    let _ = halogen_e2e::wait_for_css(d, EP_SCROLL, Duration::from_secs(8)).await;
    // Sort dropdown.
    if try_click(d, "[aria-label='Sort']").await {
        sleep(400).await;
        s.shot(d, "sort-dropdown").await;
        try_click(d, "[aria-label='Sort']").await; // close
        sleep(200).await;
    }
    // Filter funnel.
    if try_click(d, "button[aria-label='Filter']").await {
        sleep(400).await;
        s.shot(d, "filter-dropdown").await;
        try_click(d, "button[aria-label='Filter']").await;
        sleep(200).await;
    }
    // Search bar revealed.
    if try_click(d, "button[aria-label='Search']").await {
        sleep(300).await;
        let _ = halogen_e2e::fill(d, "input[placeholder='Search...']", "Episode").await;
        sleep(500).await;
        s.shot(d, "search").await;
        goto(d, base, "/latest", settle_ms).await;
    }
    // Kebab quick-menu (per-row context menu overlay).
    if try_click(d, "button[aria-label='Episode actions']").await {
        sleep(500).await;
        s.shot(d, "row-kebab-menu").await;
        // Dismiss via Escape.
        if let Ok(body) = d.find(By::Css("body")).await {
            let _ = body.send_keys(Key::Escape).await;
        }
        sleep(300).await;
    }
    // Multiselect + bulk-action menu.
    if try_click(d, "button[aria-label='Select multiple']").await {
        sleep(300).await;
        // Tick the first couple of row checkboxes.
        for _ in 0..2 {
            try_click(d, "input.checkbox").await;
        }
        sleep(300).await;
        s.shot(d, "multiselect").await;
        if try_click(d, "button[aria-label='Bulk actions']").await {
            sleep(500).await;
            s.shot(d, "bulk-menu").await;
            if let Ok(body) = d.find(By::Css("body")).await {
                let _ = body.send_keys(Key::Escape).await;
            }
            sleep(200).await;
        }
        try_click(d, "button[aria-label='Exit multiselect']").await;
        sleep(200).await;
    }

    // ── 5. Interactive gestures (swipe, pull-to-refresh) ────────────────────
    println!("gestures");
    goto(d, base, "/queue", settle_ms).await;
    let _ = halogen_e2e::wait_for_css(d, EP_SCROLL, Duration::from_secs(8)).await;
    swipe_hold(d, 0, 160.0).await; // reveal swipe-right action
    s.shot(d, "swipe-right").await;
    gesture_release(d).await;
    swipe_hold(d, 0, -160.0).await; // reveal swipe-left action
    s.shot(d, "swipe-left").await;
    gesture_release(d).await;
    pull_to_refresh_hold(d).await;
    s.shot(d, "pull-to-refresh").await;
    gesture_release(d).await;

    // ── 6. Detail pages ─────────────────────────────────────────────────────
    println!("detail pages");
    goto(d, base, "/latest", settle_ms).await;
    let _ = halogen_e2e::wait_for_css(d, EP_SCROLL, Duration::from_secs(8)).await;
    // Episode rows navigate via an `onclick` (path-string nav), not an `<a
    // href>` — click the title (the click bubbles to the row's nav handler).
    if try_click(d, "#episode-scroll h2").await {
        sleep(settle_ms).await;
        s.shot(d, "episode-detail").await;
    }
    goto(d, base, "/podcasts", settle_ms).await;
    if try_click(d, "a[href*='/podcasts/']").await {
        sleep(settle_ms).await;
        s.shot(d, "podcast-detail").await;
    }
    goto(d, base, "/playlists", settle_ms).await;
    if try_click(d, "a[href*='/playlists/']").await {
        sleep(settle_ms).await;
        s.shot(d, "playlist-detail").await;
    }
    // Discover (pre-search).
    goto(d, base, "/discover", settle_ms).await;
    s.shot(d, "discover").await;

    // ── 7. Player ───────────────────────────────────────────────────────────
    println!("player");
    goto(d, base, "/queue", settle_ms).await;
    if try_click(d, ".badge.badge-outline.cursor-pointer").await {
        // Wait for the mini player to mount + start.
        let _ = halogen_e2e::wait_for_css(d, "#mini-player", Duration::from_secs(8)).await;
        sleep(1200).await;
        s.shot(d, "mini-player").await;
        // Expand to the full-screen now-playing.
        if try_click(d, "#mini-player button.flex-1").await {
            sleep(900).await;
            s.shot(d, "now-playing").await;
            // Speed drop-up.
            if try_click(d, "[aria-label='Playback speed']").await {
                sleep(400).await;
                s.shot(d, "now-playing-speed").await;
                if let Ok(body) = d.find(By::Css("body")).await {
                    let _ = body.send_keys(Key::Escape).await;
                }
                sleep(200).await;
            }
            // Collapse back.
            try_click(d, "button[aria-label='Collapse player']").await;
            sleep(300).await;
        }
        // Pause + close so it doesn't bleed into later shots.
        try_click(d, "#mini-player button[aria-label='Pause']").await;
        try_click(d, "#mini-player button[aria-label='Close player']").await;
        sleep(300).await;
    }

    // ── 8. Download states (on Latest) ──────────────────────────────────────
    println!("download states");
    goto(d, base, "/latest", settle_ms).await;
    let _ = halogen_e2e::wait_for_css(d, EP_SCROLL, Duration::from_secs(8)).await;
    // The idle "Download to device" and the on-device "remove" badge both appear
    // in the list (some rows were staged on-device above).
    s.shot(d, "download-badges").await;

    // ── 9. Settings & admin ─────────────────────────────────────────────────
    println!("settings + admin");
    for (route, name) in [
        ("/settings", "settings"),
        ("/settings/playback", "settings-playback"),
        ("/settings/downloads", "settings-downloads"),
        ("/settings/ui", "settings-ui"),
        ("/settings/accounts", "settings-accounts"),
        ("/settings/server", "settings-server"),
        ("/settings/dock", "settings-dock"),
        ("/settings/configure-swipes", "settings-swipes"),
        ("/polling", "polling"),
        ("/logs/device", "logs-device"),
        ("/admin/users", "admin-users"),
        ("/admin/logs", "admin-logs"),
        ("/admin/errors", "admin-errors"),
        ("/settings/config", "view-config"),
        ("/cache-control", "cache-control"),
        ("/menu", "menu"),
    ] {
        goto(d, base, route, settle_ms).await;
        s.shot(d, name).await;
    }

    // ── 10. Chrome: offline + error states ──────────────────────────────────
    println!("offline + errors");
    goto(d, base, "/queue", settle_ms).await;
    // Toggle "Go offline" via the navbar status button (label is lowercase).
    if try_click(d, "button[aria-label='Go offline']").await {
        sleep(900).await;
        s.shot(d, "navbar-offline").await;
        // Queue offline-unavailable message (fresh, unresolved queue offline).
        goto(d, base, "/queue", settle_ms).await;
        s.shot(d, "queue-offline").await;
        try_click(d, "button[aria-label='Go online']").await;
        sleep(600).await;
    }
    // 404.
    goto(d, base, "/no-such-page", settle_ms).await;
    s.shot(d, "not-found").await;

    Ok(())
}

// ── main ──────────────────────────────────────────────────────────────────────

/// Compute the run directory `design/screenshots/web/<datetime>/`, resolving `data/`
/// relative to the repo (or `--out`). Creates it.
fn run_dir(out_base: Option<PathBuf>) -> Result<(PathBuf, PathBuf)> {
    let base = out_base.unwrap_or_else(|| {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../design/screenshots/web")
    });
    let stamp = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S").to_string();
    let dir = base.join(&stamp);
    std::fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    Ok((base, dir))
}

/// Refresh `<base>/latest/` to mirror the just-finished run.
fn mirror_latest(base: &Path, run: &Path) -> Result<()> {
    let latest = base.join("latest");
    let _ = std::fs::remove_dir_all(&latest);
    std::fs::create_dir_all(&latest)?;
    for entry in std::fs::read_dir(run)?.flatten() {
        let p = entry.path();
        if p.is_file() {
            std::fs::copy(&p, latest.join(entry.file_name()))?;
        }
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = parse_args();
    let (out_base, run) = run_dir(args.out.clone())?;
    let shooter = Shooter {
        dir: run.clone(),
        n: AtomicUsize::new(0),
    };
    let settle_ms = 1800;

    // Resolve target + credentials.
    let (base_url, user, pass, on_device, _app): (
        String,
        String,
        String,
        Vec<i32>,
        Option<TestApp>,
    ) = if let Some(url) = args.base_url.clone() {
        let user = args
            .user
            .clone()
            .context("--user (or SHOT_USER) required with --base-url")?;
        let pass = args
            .pass
            .clone()
            .context("--pass (or SHOT_PASS) required with --base-url")?;
        (url, user, pass, Vec::new(), None)
    } else {
        let dist = dist_dir().context(
            "no built dist/ — run `just ui-build` first (or pass --base-url for a live server)",
        )?;
        println!("spawning seeded server over {}", dist.display());
        let app = support::spawn_with(SpawnOptions {
            public_root: Some(dist),
            use_mock_download: true,
            ..Default::default()
        })
        .await;
        let admin = app.seed_admin().await;
        let on_device = seed(&app, admin.id).await;
        (
            app.base_url.clone(),
            admin.username.clone(),
            admin.password.clone(),
            on_device,
            Some(app),
        )
    };

    let Some((_driver_guard, driver)) = halogen_e2e::browser_session().await else {
        eprintln!("no chromedriver/Chrome available — cannot capture screenshots");
        std::process::exit(1);
    };

    apply_viewport(&driver).await;

    let result = run_flow(
        &shooter, &driver, &base_url, &user, &pass, &on_device, settle_ms,
    )
    .await;
    let _ = driver.quit().await;

    let total = shooter.n.load(Ordering::SeqCst);
    if let Err(e) = mirror_latest(&out_base, &run) {
        eprintln!("warning: could not refresh latest/ ({e})");
    }
    result.context("screenshot walk")?;

    println!("\n{total} screenshots → {}", run.display());
    println!("latest → {}", out_base.join("latest").display());
    Ok(())
}
