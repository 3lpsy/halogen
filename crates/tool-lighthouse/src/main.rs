//! `tool-lighthouse` — drive a real Chrome through every feature of the app and
//! collect Web Vitals per step (pure Rust, no Node).
//!
//! Two modes:
//!   * **Self-hosted (default):** spawn a real, seeded axum server serving the
//!     built `dist/` (run `just ui-build` first), log in as the seeded admin, and
//!     walk the app. Hermetic.
//!   * **External (`--base-url`):** point at a running instance (e.g. the dev pod);
//!     supply `--user`/`--pass` (or `LH_USER`/`LH_PASS`).
//!
//! Needs chromedriver + Chrome on PATH (same prerequisites as `just e2e`). See the
//! crate `Cargo.toml` header for how this differs from the Lighthouse CLI.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use halogen_integ::{SpawnOptions, TestApp, spawn_with};
use thirtyfour::prelude::*;

mod vitals;
use vitals::StepVitals;

struct Args {
    base_url: Option<String>,
    user: Option<String>,
    pass: Option<String>,
    out: PathBuf,
    episodes: usize,
    /// CDP CPU throttling multiplier (Lighthouse mobile uses 4). `1.0` disables —
    /// on fast desktops long tasks rarely cross the 50ms bar without this, so
    /// blocking time / INP read as ~0.
    cpu: f64,
    /// Also apply Lighthouse-ish mobile network throttling (slower; off by default).
    throttle_network: bool,
    /// Force the read-only guard on (blocks server-download triggers + audio
    /// fetches). Defaults on automatically for `--base-url`; this forces it for
    /// self-hosted runs too.
    read_only: bool,
    /// Disable the read-only guard even against `--base-url` (lets the app's
    /// auto-download fire — i.e. permit its normal write side-effects).
    allow_writes: bool,
}

fn parse_args() -> Args {
    let mut args = Args {
        base_url: None,
        user: std::env::var("LH_USER").ok(),
        pass: std::env::var("LH_PASS").ok(),
        out: PathBuf::from(
            std::env::var("LH_OUT").unwrap_or_else(|_| "target/lighthouse".to_string()),
        ),
        episodes: 45,
        cpu: 4.0,
        throttle_network: false,
        read_only: false,
        allow_writes: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--base-url" => args.base_url = it.next(),
            "--user" => args.user = it.next(),
            "--pass" => args.pass = it.next(),
            "--out" => {
                if let Some(v) = it.next() {
                    args.out = PathBuf::from(v);
                }
            }
            "--episodes" => {
                if let Some(v) = it.next().and_then(|v| v.parse().ok()) {
                    args.episodes = v;
                }
            }
            "--cpu" => {
                if let Some(v) = it.next().and_then(|v| v.parse().ok()) {
                    args.cpu = v;
                }
            }
            "--throttle-network" => args.throttle_network = true,
            "--read-only" => args.read_only = true,
            "--allow-writes" => args.allow_writes = true,
            "-h" | "--help" => {
                eprintln!(
                    "tool-lighthouse — Web-Vitals walkthrough of the whole app\n\n\
                     USAGE: tool-lighthouse [--base-url URL --user U --pass P] [--out DIR]\n\
                     \x20                  [--episodes N] [--cpu RATE] [--throttle-network]\n\
                     \x20                  [--read-only] [--allow-writes]\n\n\
                     Default (no --base-url): spawn a seeded server over the built dist/.\n\
                     --cpu RATE: CDP CPU throttle (default 4, like Lighthouse mobile; 1 = off).\n\
                     --read-only: block server-download triggers + audio fetches (auto-on for\n\
                     \x20            --base-url so a prod run has no write side-effects; login aside).\n\
                     --allow-writes: disable that guard even against --base-url.\n\
                     Needs chromedriver + Chrome (set CHROMEDRIVER/WEBDRIVER_URL/CHROME as for `just e2e`)."
                );
                std::process::exit(0);
            }
            other => eprintln!("warning: ignoring unknown arg {other:?}"),
        }
    }
    args
}

/// Apply CDP mobile device metrics + (optional) CPU/network throttling so the run
/// resembles Lighthouse's mobile profile. Best-effort: a non-Chromium driver just
/// logs a warning and continues unthrottled.
async fn apply_emulation(d: &WebDriver, cpu: f64, throttle_network: bool) {
    let cdp = d.cdp();
    if let Err(e) = cdp
        .send_raw(
            "Emulation.setDeviceMetricsOverride",
            serde_json::json!({
                "width": 412, "height": 823, "deviceScaleFactor": 1.75,
                "mobile": true, "screenWidth": 412, "screenHeight": 823
            }),
        )
        .await
    {
        eprintln!("warning: mobile device metrics override failed ({e}); staying desktop");
    }
    if cpu > 1.0
        && let Err(e) = cdp
            .send_raw(
                "Emulation.setCPUThrottlingRate",
                serde_json::json!({ "rate": cpu }),
            )
            .await
    {
        eprintln!(
            "warning: CPU throttle failed ({e}); running unthrottled — blocking/INP will read low"
        );
    }
    if throttle_network {
        // Lighthouse mobile "Slow 4G"-ish: ~150ms RTT, 1.6 Mbps down / 0.75 up.
        let _ = cdp
            .send_raw(
                "Network.emulateNetworkConditions",
                serde_json::json!({
                    "offline": false, "latency": 150.0,
                    "downloadThroughput": 1.6 * 1024.0 * 1024.0 / 8.0,
                    "uploadThroughput": 0.75 * 1024.0 * 1024.0 / 8.0
                }),
            )
            .await;
    }
}

/// Block the app's only server-mutating requests — the download triggers
/// (`/episodes/download/bulk`) — plus the big audio byte fetches, via CDP URL
/// blocking. Reads (episode/podcast/playlist lists, art, download-progress) are
/// untouched. Fails the run rather than proceed unprotected, so "read-only" is a
/// guarantee, not best-effort.
async fn apply_read_only(d: &WebDriver) -> Result<()> {
    let cdp = d.cdp();
    cdp.send_raw("Network.enable", serde_json::json!({}))
        .await
        .context("Network.enable (read-only guard)")?;
    cdp.send_raw(
        "Network.setBlockedURLs",
        serde_json::json!({ "urls": [
            "*/episodes/download/bulk*",  // server-download trigger + bulk remove (POST/DELETE)
            "*/episodes/*/download",      // per-episode server-download (POST/DELETE)
            "*/episodes/*/audio*",        // device-download byte stream (large GET; not needed)
        ]}),
    )
    .await
    .context("Network.setBlockedURLs (read-only guard)")?;
    Ok(())
}

fn dist_dir() -> Option<PathBuf> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../dist")
        .canonicalize()
        .ok()?;
    dir.join("index.html").is_file().then_some(dir)
}

async fn sleep(ms: u64) {
    tokio::time::sleep(Duration::from_millis(ms)).await;
}

/// Seed enough content that every feature page has real rows to render + scroll.
async fn seed_content(app: &TestApp, admin_id: i32, episodes: usize) {
    let p1 = app
        .seed_podcast("Tech Talk Weekly", "https://feed.test/tech")
        .await;
    let e1 = app.seed_episodes(p1, episodes).await;
    let p2 = app
        .seed_podcast("Science Hour", "https://feed.test/science")
        .await;
    let _e2 = app.seed_episodes(p2, episodes).await;

    let take = |n: usize| &e1[..e1.len().min(n)];
    let queue = app.seed_playlist("Queue", true).await;
    app.seed_playlist_episodes(queue, take(12)).await;
    let favorites = app.seed_playlist("Favorites", false).await;
    app.seed_playlist_episodes(favorites, take(8)).await;
    app.seed_playbacks(admin_id, take(15)).await;
}

/// A page-load step: (re)install the collector (which starts the frame-rate row
/// logger), let the SPA boot + settle, then drain. For list pages the drain prints
/// a `rowlog:` line showing the sub-frame row-count / remount sequence.
async fn measure_load(d: &WebDriver, name: &str, settle_ms: u64) -> Result<StepVitals> {
    vitals::install(d).await?;
    sleep(settle_ms).await;
    vitals::drain(d, name, true).await
}

/// Scroll a container to the bottom in `steps` increments (tripping infinite-scroll
/// and forcing per-frame render), then back to the top. Best-effort — a missing
/// container just no-ops.
async fn scroll_through(d: &WebDriver, sel: &str, steps: usize) {
    let to_bottom = "const el = document.querySelector(arguments[0]); if (el) { el.scrollTop = el.scrollHeight; }";
    let to_top = "const el = document.querySelector(arguments[0]); if (el) { el.scrollTop = 0; }";
    for _ in 0..steps {
        let _ = d
            .execute(to_bottom, vec![serde_json::Value::String(sel.to_string())])
            .await;
        sleep(500).await;
    }
    let _ = d
        .execute(to_top, vec![serde_json::Value::String(sel.to_string())])
        .await;
    sleep(300).await;
}

/// Open the search bar and type a query (best-effort).
async fn search(d: &WebDriver, query: &str) {
    if let Ok(btn) = d.find(By::Css("button[aria-label='Search']")).await {
        let _ = btn.click().await;
        sleep(400).await;
    }
    if let Ok(input) = d
        .query(By::Css("input[placeholder='Search...']"))
        .first()
        .await
    {
        let _ = input.send_keys(query).await;
    }
    sleep(1200).await;
}

/// Click the first episode-detail link on the page (best-effort).
async fn open_first_episode(d: &WebDriver) {
    if let Ok(link) = d.query(By::Css("a[href*='/episodes/']")).first().await {
        let _ = link.click().await;
    }
    sleep(1800).await;
}

/// The deployed WASM URL (e.g. `/assets/halogen-ui-<hash>_bg.wasm`), so each run
/// records exactly which build it tested. Prefer the `.wasm` (all the Rust code +
/// inline styles live there and its hash changes on every source edit); the JS
/// glue filename can stay identical across Rust changes, so it's a poor signal.
async fn probe_build(d: &WebDriver) -> String {
    let script = "const r = performance.getEntriesByType('resource').map(e => e.name);\
        return r.find(n => /\\.wasm(\\?|$)/.test(n)) \
            || r.find(n => /\\/assets\\/halogen-ui-.*\\.js/.test(n)) \
            || '';";
    d.execute(script, Vec::new())
        .await
        .ok()
        .and_then(|r| r.json().as_str().map(str::to_string))
        .unwrap_or_default()
}

/// Walk every feature, returning per-step vitals + the tested bundle URL.
/// `settle_ms` is the per-load wait (scaled up under CPU throttle so the wasm
/// finishes booting before we drain).
async fn run_flow(
    d: &WebDriver,
    base: &str,
    user: &str,
    pass: &str,
    settle_ms: u64,
) -> Result<(Vec<StepVitals>, String)> {
    let mut steps = Vec::new();

    // Cold load of the login page.
    d.goto(format!("{base}/auth/login"))
        .await
        .context("goto login")?;
    steps.push(measure_load(d, "Cold load — login", settle_ms).await?);
    let build = probe_build(d).await;

    // Sign in (setup — the post-login destination pages are measured directly).
    halogen_e2e::login_via_ui(d, base, user, pass).await;
    sleep(settle_ms).await;

    // Feature pages: load + (where scrollable) a scroll window.
    let pages: &[(&str, &str, Option<&str>)] = &[
        ("/latest", "Latest", Some("#episode-scroll")),
        ("/podcasts", "Podcasts", Some("#podcast-scroll")),
        ("/playlists", "Playlists", Some("#playlist-scroll")),
        ("/queue", "Queue", Some("#episode-scroll")),
        ("/history", "History", Some("#episode-scroll")),
        ("/downloads", "Downloads", Some("#episode-scroll")),
        ("/discover", "Discover", None),
    ];
    for (route, name, scroll) in pages {
        d.goto(format!("{base}{route}"))
            .await
            .with_context(|| format!("goto {route}"))?;
        steps.push(measure_load(d, &format!("{name} — load"), settle_ms).await?);
        if let Some(sel) = scroll {
            vitals::baseline(d).await?;
            scroll_through(d, sel, 8).await;
            steps.push(vitals::drain(d, &format!("{name} — scroll"), false).await?);
            if *route == "/latest" {
                vitals::baseline(d).await?;
                search(d, "a").await;
                steps.push(vitals::drain(d, "Latest — search", false).await?);
            }
        }
    }

    // Opening an episode-detail page.
    d.goto(format!("{base}/latest"))
        .await
        .context("goto latest")?;
    vitals::install(d).await?;
    sleep(settle_ms).await;
    vitals::baseline(d).await?;
    open_first_episode(d).await;
    steps.push(vitals::drain(d, "Open an episode", false).await?);

    Ok((steps, build))
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = parse_args();
    std::fs::create_dir_all(&args.out).context("create --out dir")?;
    let out = args.out.canonicalize().context("canonicalize --out")?;

    // Resolve the target + credentials, keeping the spawned server (if any) alive
    // for the whole run by binding it to `_app`.
    let _app: Option<TestApp>;
    let (base_url, user, pass);
    if let Some(b) = args.base_url.clone() {
        let u = args
            .user
            .clone()
            .context("--base-url mode needs --user (or LH_USER)")?;
        let p = args
            .pass
            .clone()
            .context("--base-url mode needs --pass (or LH_PASS)")?;
        (base_url, user, pass) = (b, u, p);
        _app = None;
    } else {
        let dist =
            dist_dir().context("no built dist/ — run `just ui-build` first (or use --base-url)")?;
        eprintln!("→ spawning seeded server over {}", dist.display());
        let app = spawn_with(SpawnOptions {
            public_root: Some(dist),
            use_mock_download: true,
            ..Default::default()
        })
        .await;
        let admin = app.seed_admin().await;
        seed_content(&app, admin.id, args.episodes).await;
        base_url = app.base_url.clone();
        user = admin.username.clone();
        pass = admin.password.clone();
        _app = Some(app);
    }

    // Browser: reuse the E2E tier's chromedriver lifecycle + headless connect.
    let Some((_chromedriver, driver)) = halogen_e2e::browser_session().await else {
        bail!(
            "no chromedriver available — install chromedriver + Chrome \
             (or set CHROMEDRIVER / WEBDRIVER_URL), same as `just e2e`"
        );
    };

    // Mobile + CPU/network throttle so blocking time, INP and CLS resemble a
    // Lighthouse mobile run (best-effort).
    apply_emulation(&driver, args.cpu, args.throttle_network).await;

    // Read-only guard: on by default for an external target (a prod run should
    // have no write side-effects beyond login), forceable with --read-only, and
    // opt-out with --allow-writes.
    let read_only = !args.allow_writes && (args.read_only || args.base_url.is_some());
    if read_only {
        apply_read_only(&driver)
            .await
            .context("enable read-only guard (use --allow-writes to bypass)")?;
    }

    // Settle longer when the CPU is throttled so the (now slower) wasm boot
    // finishes before we drain each load window.
    let settle_ms = 2500 + ((args.cpu - 1.0).max(0.0) * 800.0) as u64;
    eprintln!(
        "→ walking {base_url} (cpu x{:.0}{}{}, {settle_ms}ms settle)",
        args.cpu,
        if args.throttle_network {
            ", net-throttled"
        } else {
            ""
        },
        if read_only {
            ", read-only"
        } else {
            ", WRITES ALLOWED"
        }
    );
    let result = run_flow(&driver, &base_url, &user, &pass, settle_ms).await;
    // Always quit the browser, even if the walk failed partway.
    let _ = driver.quit().await;
    let (steps, build) = result?;

    if build.is_empty() {
        eprintln!("→ tested build: (could not detect bundle URL)");
    } else {
        eprintln!("→ tested build: {build}");
    }
    vitals::write_reports(&out, &base_url, &build, &steps)?;
    vitals::print_summary(&base_url, &steps);
    println!(
        "\nReports written:\n  {}\n  {}",
        out.join("report.md").display(),
        out.join("report.json").display()
    );
    Ok(())
}
