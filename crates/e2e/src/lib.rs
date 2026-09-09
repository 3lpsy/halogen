//! Browser tests build the WASM frontend, serve it through the real Axum app, and drive it with thirtyfour; only
//! upstream RSS is mocked. ChromeDriver owns an ephemeral driver process and kills it on drop, or WEBDRIVER_URL selects
//! an existing driver.

mod screenshot;
pub use screenshot::{SCREENSHOT_DIR_ENV, shot};

use std::io;
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use futures::FutureExt;
use halogen_integ::support;
use thirtyfour::prelude::*;
use thirtyfour::{BrowserLogEntry, LoggingPrefsLogLevel};
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Locate the built frontend (`<workspace>/dist`) if it exists and looks built.
/// Returns `None` when there's no `index.html`, so tests can skip cleanly
/// instead of failing on an un-built frontend.
pub fn dist_dir() -> Option<PathBuf> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../dist")
        .canonicalize()
        .ok()?;
    dir.join("index.html").is_file().then_some(dir)
}

/// Skip-guard for the browser tests: returns `true` when `dist/` is built, else
/// prints the standard skip reason and returns `false` so the caller can bail
/// (`if !require_dist() { return; }`). Replaces the per-test `dist_dir().is_none()`
/// boilerplate.
pub fn require_dist() -> bool {
    if dist_dir().is_some() {
        return true;
    }
    assert!(
        !is_e2e_required(),
        "browser gate requires built dist/: run just ui-build"
    );
    eprintln!("skipping: no built dist/ — run `just ui-build` first");
    false
}

/// A `chromedriver` child process bound to an ephemeral port. Killed on drop so
/// every test run leaves no stray daemon behind.
pub struct ChromeDriver {
    child: Child,
    port: u16,
}

impl ChromeDriver {
    /// Launch `chromedriver` on a free port and wait until it accepts
    /// connections. The binary name can be overridden with `CHROMEDRIVER`.
    /// Returns `Err` (e.g. `NotFound`) when the binary is absent, so callers can
    /// skip rather than fail.
    pub fn start() -> io::Result<Self> {
        let port = free_port()?;
        let bin = std::env::var("CHROMEDRIVER").unwrap_or_else(|_| "chromedriver".to_string());

        let child = Command::new(bin)
            .arg(format!("--port={port}"))
            .arg("--silent")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;

        let driver = ChromeDriver { child, port };
        driver.wait_ready(Duration::from_secs(10))?;
        Ok(driver)
    }

    /// The WebDriver endpoint URL for this driver.
    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    fn wait_ready(&self, timeout: Duration) -> io::Result<()> {
        let addr = SocketAddrV4::new(Ipv4Addr::LOCALHOST, self.port);
        let deadline = Instant::now() + timeout;
        loop {
            if TcpStream::connect(addr).is_ok() {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "chromedriver did not become ready in time",
                ));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

impl Drop for ChromeDriver {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Ask the OS for a free TCP port by binding to `:0` and releasing it. There's a
/// small race before chromedriver re-binds, but it's fine for test setup.
fn free_port() -> io::Result<u16> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    Ok(listener.local_addr()?.port())
}

/// Candidate Chrome/Chromium binary names (Fedora ships `chromium-browser`).
const CHROME_BINS: &[&str] = &[
    "chromium-browser",
    "chromium",
    "google-chrome",
    "google-chrome-stable",
    "chrome",
];

/// Locate a Chrome/Chromium binary: `CHROME_BIN` env first, then PATH lookups.
fn chrome_binary() -> Option<String> {
    if let Ok(p) = std::env::var("CHROME_BIN")
        && !p.is_empty()
    {
        return Some(p);
    }
    for name in CHROME_BINS {
        if let Ok(out) = Command::new("sh")
            .arg("-c")
            .arg(format!("command -v {name}"))
            .output()
            && out.status.success()
        {
            let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !path.is_empty() {
                return Some(path);
            }
        }
    }
    None
}

/// Connect a headless-Chrome session to a WebDriver `endpoint`. Sets the browser binary explicitly
/// (chromedriver otherwise looks only for `google-chrome` and hangs on distros that ship `chromium-browser`),
/// plus a unique throwaway profile and the usual containerized-Chrome hardening flags.
pub async fn connect(endpoint: &str) -> WebDriverResult<WebDriver> {
    let mut caps = DesiredCapabilities::chrome();
    if let Some(bin) = chrome_binary() {
        caps.set_binary(&bin)?;
    }
    caps.add_arg("--headless=new")?;
    // Containerized / root Chrome hardening.
    caps.add_arg("--no-sandbox")?;
    caps.add_arg("--disable-dev-shm-usage")?;
    caps.add_arg("--disable-gpu")?;
    // Playback starts after async gaps (device-download → play), where the
    // user-gesture context is lost — don't let autoplay policy reject play().
    caps.add_arg("--autoplay-policy=no-user-gesture-required")?;
    // New headless can route decoded audio to the host's real output device —
    // playback journeys would otherwise blast the dev's speakers/headphones.
    // Mute at the browser level (decoding/`timeupdate` still run, so playback
    // assertions are unaffected).
    caps.add_arg("--mute-audio")?;
    // Capture page-side `console.*` (the wasm app's `tracing` → WASMLayer feeds
    // it) so [`browser_logs`] can dump the app's own logs on failure. Without
    // this capability chromedriver's `/log` endpoint returns nothing.
    caps.set_browser_log_level(LoggingPrefsLogLevel::All)?;
    // Unique profile dir per session so localStorage/auth never leaks between
    // tests and locked default profiles can't deadlock.
    let n = SESSION.fetch_add(1, Ordering::SeqCst);
    let profile =
        std::env::temp_dir().join(format!("halogen-e2e-profile-{}-{}", std::process::id(), n));
    caps.add_arg(&format!("--user-data-dir={}", profile.display()))?;
    WebDriver::new(endpoint, caps).await
}

static SESSION: AtomicU32 = AtomicU32::new(0);

/// Connect to an externally-managed WebDriver (the `WEBDRIVER_URL` env var, or
/// chromedriver's default `http://localhost:9515`). Use [`ChromeDriver::start`]
/// + [`connect`] when you want the test to own the driver lifecycle.
pub async fn headless_chrome() -> WebDriverResult<WebDriver> {
    let url =
        std::env::var("WEBDRIVER_URL").unwrap_or_else(|_| "http://localhost:9515".to_string());
    connect(&url).await
}

/// Open a browser session, owning the chromedriver lifecycle. Returns `None` (with a printed reason) only when
/// the toolchain is genuinely absent — `WEBDRIVER_URL` unset *and* no `chromedriver` binary — so tests can skip
/// on a bare machine. If chromedriver is present but the browser fails to launch, that's a real error and
/// panics.
pub async fn browser_session() -> Option<(Option<ChromeDriver>, WebDriver)> {
    if std::env::var("WEBDRIVER_URL").is_ok() {
        return Some((
            None,
            headless_chrome().await.expect("connect external WebDriver"),
        ));
    }
    match ChromeDriver::start() {
        Ok(driver) => {
            let session = connect(&driver.url())
                .await
                .expect("open headless browser session");
            Some((Some(driver), session))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            assert!(
                !is_e2e_required(),
                "browser gate requires chromedriver: {e}"
            );
            eprintln!("skipping: no chromedriver on PATH (set CHROMEDRIVER/WEBDRIVER_URL) — {e}");
            None
        }
        Err(e) => panic!("chromedriver failed to start: {e}"),
    }
}

// ── Browser interaction helpers (used by journey tests) ──────────────────────

/// Wait until the current URL contains `needle`. Returns false on timeout.
pub async fn wait_for_url(driver: &WebDriver, needle: &str, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if let Ok(u) = driver.current_url().await
            && u.as_str().contains(needle)
        {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
}

/// Wait until an element matching `css` exists. Returns false on timeout.
pub async fn wait_for_css(driver: &WebDriver, css: &str, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if driver.find(By::Css(css)).await.is_ok() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
}

/// Wait until the page body text contains `needle`. Returns false on timeout.
pub async fn wait_for_text(driver: &WebDriver, needle: &str, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if let Ok(body) = driver.find(By::Tag("body")).await
            && let Ok(t) = body.text().await
            && t.contains(needle)
        {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
}

/// Wait until the page body text NO LONGER contains `needle`. Returns false on
/// timeout. The counterpart to [`wait_for_text`], for asserting a row/section
/// went away after an async action — polling for absence is the only way to
/// synchronize with a removal, since there's no element to wait ON.
pub async fn wait_for_text_gone(driver: &WebDriver, needle: &str, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if let Ok(body) = driver.find(By::Tag("body")).await
            && let Ok(t) = body.text().await
            && !t.contains(needle)
        {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
}

/// Type `text` into the first input matching `css` (clears focus first).
pub async fn fill(driver: &WebDriver, css: &str, text: &str) -> WebDriverResult<()> {
    let el = driver.query(By::Css(css)).first().await?;
    el.click().await.ok();
    // Clear any existing value first — `send_keys` appends at the cursor, so a
    // pre-populated field (e.g. the login page prefills the server URL with
    // the page origin on wasm) would otherwise concatenate into a malformed value.
    el.clear().await.ok();
    el.send_keys(text).await?;
    Ok(())
}

/// Scroll and click, falling back to JS for intercepted or not-interactable targets. Open dropdowns and transitioning
/// context menus can obstruct native hit testing. Disabled controls still reject clicks.
pub async fn click_el(driver: &WebDriver, el: &WebElement) -> WebDriverResult<()> {
    el.scroll_into_view().await.ok();
    // `WebDriverError` is a newtype over `WebDriverErrorInner`; the variant lives
    // on the inner enum (the same-named assoc fn on the outer type is a
    // constructor, not a pattern), so inspect via `as_inner()`.
    match el.click().await {
        Err(e)
            if matches!(
                e.as_inner(),
                thirtyfour::error::WebDriverErrorInner::ElementClickIntercepted(_)
                    | thirtyfour::error::WebDriverErrorInner::ElementNotInteractable(_)
            ) =>
        {
            driver
                .execute("arguments[0].click();", vec![el.to_json()?])
                .await?;
            Ok(())
        }
        other => other,
    }
}

/// Click the first element matching `css` (overlay-tolerant — see [`click_el`]). Re-finds on staleness: a
/// list/detail re-render between the find and the click (e.g. a device-download progress tick replacing the
/// Play button) can invalidate the element handle (`StaleElementReference`). Re-query + retry a few times so a
/// mid-render click doesn't spuriously fail the journey.
pub async fn click(driver: &WebDriver, css: &str) -> WebDriverResult<()> {
    let mut last_err = None;
    for _ in 0..6 {
        let el = driver.query(By::Css(css)).first().await?;
        match click_el(driver, &el).await {
            Ok(()) => return Ok(()),
            Err(e)
                if matches!(
                    e.as_inner(),
                    thirtyfour::error::WebDriverErrorInner::StaleElementReference(_)
                ) =>
            {
                last_err = Some(e);
                tokio::time::sleep(Duration::from_millis(150)).await;
            }
            Err(e) => return Err(e),
        }
    }
    Err(last_err.expect("loop runs at least once"))
}

/// Click the first `<button>` whose trimmed visible text equals `text`. Used for
/// menu/dropdown actions that are keyed by label rather than a stable selector
/// (e.g. the quick context menu's "Add to queue", the sort dropdown's fields).
pub async fn click_button_text(driver: &WebDriver, text: &str) -> WebDriverResult<()> {
    let xpath = format!("//button[normalize-space()='{text}']");
    let el = driver.query(By::XPath(xpath)).first().await?;
    // Overlay/slide-in tolerant — menu actions live in the always-mounted quick
    // panel that animates in (see [`click_el`]).
    click_el(driver, &el).await
}

/// Number of elements matching `css` right now (0 if none / on error).
pub async fn count(driver: &WebDriver, css: &str) -> usize {
    driver
        .find_all(By::Css(css))
        .await
        .map(|v| v.len())
        .unwrap_or(0)
}

/// Wait until at least `min` elements match `css`, returning the final count.
pub async fn wait_for_count(driver: &WebDriver, css: &str, min: usize, timeout: Duration) -> usize {
    let deadline = Instant::now() + timeout;
    loop {
        let n = count(driver, css).await;
        if n >= min || Instant::now() >= deadline {
            return n;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// Wait until exactly `target` elements match `css` and the count holds steady for one extra poll (so a list
/// mid-transition isn't sampled early). Returns the last observed count — equal to `target` on success,
/// otherwise whatever it was when the timeout hit. Use for filter/search assertions where the set both grows
/// and shrinks.
pub async fn wait_for_count_eq(
    driver: &WebDriver,
    css: &str,
    target: usize,
    timeout: Duration,
) -> usize {
    let deadline = Instant::now() + timeout;
    let mut last = count(driver, css).await;
    loop {
        tokio::time::sleep(Duration::from_millis(200)).await;
        let n = count(driver, css).await;
        if n == target && last == target {
            return n;
        }
        last = n;
        if Instant::now() >= deadline {
            return n;
        }
    }
}

/// Drain Chrome browser logs, including WASM tracing, one entry per line. Default verbosity is INFO; rebuild the
/// frontend with RUST_LOG=halogen_webui=debug for more. Empty buffers or unsupported driver logging return an empty
/// string.
pub async fn browser_logs(driver: &WebDriver) -> String {
    match driver.get_log("browser").await {
        Ok(entries) => entries
            .iter()
            .filter_map(clean_browser_log)
            .collect::<Vec<_>>()
            .join("\n"),
        Err(_) => String::new(),
    }
}

/// Keep WASM tracing and SEVERE browser errors, dropping framework warnings. Unwrap WASMLayer color formatting into
/// plain LEVEL, target, and message text.
fn clean_browser_log(e: &BrowserLogEntry) -> Option<String> {
    // WASMLayer format: `<url> <line:col> "%cLEVEL%c <target>%c <msg>" "color:…" …`
    if let Some(open) = e.message.find("\"%c") {
        let after_quote = &e.message[open + 1..];
        // The formatted string ends where the trailing CSS args begin (`" "`).
        let formatted = after_quote.split("\" \"").next().unwrap_or(after_quote);
        let cleaned = formatted.replace("%c", " ");
        let cleaned = cleaned.trim().trim_end_matches('"').trim();
        return (!cleaned.is_empty()).then(|| cleaned.to_string());
    }
    // Real errors (CSP blocks, uncaught JS) are worth keeping verbatim.
    (e.level == "SEVERE").then(|| format!("SEVERE {}", e.message))
}

/// Run a browser body, catch assertion panics, then finish_journey to dump console logs, quit the driver, and propagate
/// the result. Every browser test uses this teardown, including failures.
pub async fn run_session<F>(driver: WebDriver, what: &str, body: F)
where
    F: AsyncFnOnce(&WebDriver) -> WebDriverResult<()>,
{
    screenshot::set_journey(what);
    let outcome = std::panic::AssertUnwindSafe(body(&driver))
        .catch_unwind()
        .await;
    finish_journey(driver, outcome, what).await;
}

/// Dump WASM console logs to stderr, quit the driver, and propagate the catch_unwind outcome, including WebDriver
/// errors and assertion panics. Prefer run_session unless teardown needs extra steps.
pub async fn finish_journey(
    driver: WebDriver,
    outcome: std::thread::Result<WebDriverResult<()>>,
    what: &str,
) {
    let closing = if matches!(outcome, Ok(Ok(()))) {
        "99-final"
    } else {
        "99-failure"
    };
    screenshot::set_journey(what);
    screenshot::shot(&driver, closing).await;
    let logs = browser_logs(&driver).await;
    if !logs.is_empty() {
        eprintln!(
            "\n─── browser console log (wasm tracing) ───\n{logs}\n─── end browser console log ───\n"
        );
    }
    driver.quit().await.ok();
    match outcome {
        Ok(Ok(())) => {}
        Ok(Err(e)) => panic!("{what}: {e}"),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

/// The full visible text of the page body (empty string if unavailable).
pub async fn body_text(driver: &WebDriver) -> String {
    match driver.find(By::Tag("body")).await {
        Ok(b) => b.text().await.unwrap_or_default(),
        Err(_) => String::new(),
    }
}

// IndexedDB stores JSON strings: registry accounts in halogen.accounts/registry/accounts and per-account config in
// halogen.config.{segment}/kv/client_config. Resolve the active account rather than hardcoding its namespace.

/// Resolve `seg` from rawAccounts and parsed registry `a`, matching platform::namespace::segment_for: e{id} locally,
/// u{id}-{server:016x} remotely, anon otherwise. Read the u64 active_server digits from raw JSON into BigInt because
/// JSON.parse can round values above Number.MAX_SAFE_INTEGER.
const RESOLVE_SEGMENT_JS: &str = r#"
    const _m = /"active_server"\s*:\s*(\d+)/.exec(rawAccounts);
    const _srv = (_m ? BigInt(_m[1]) : 0n).toString(16).padStart(16, '0');
    const seg = (a.active_user_id === null || a.active_user_id === undefined)
        ? 'anon'
        : (a.active_kind === 'Embedded'
            ? 'e' + a.active_user_id
            : 'u' + a.active_user_id + '-' + _srv);
"#;

/// Open the active account's config DB and readwrite kv transaction, parsing client_config into c for the caller;
/// cb/tx/store/cdb remain in scope. Report false on failure so patch_active_config panics instead of silently patching
/// a newly created, wrong-namespace DB.
const PATCH_CONFIG_PREFIX: &str = r#"
    const cb = arguments[arguments.length - 1];
    const ar0 = indexedDB.open('halogen.accounts');
    ar0.onerror = () => cb(false);
    ar0.onsuccess = () => {
        const adb = ar0.result;
        if (!adb.objectStoreNames.contains('registry')) { adb.close(); cb(false); return; }
        const gr0 = adb.transaction('registry', 'readonly').objectStore('registry').get('accounts');
        gr0.onerror = () => { adb.close(); cb(false); };
        gr0.onsuccess = () => {
            adb.close();
            const rawAccounts = gr0.result || '{}';
            let a = {}; try { a = JSON.parse(rawAccounts); } catch (e) {}
            __RESOLVE_SEGMENT__
            const cr = indexedDB.open('halogen.config.' + seg);
            cr.onerror = () => cb(false);
            cr.onsuccess = () => {
                const cdb = cr.result;
                if (!cdb.objectStoreNames.contains('kv')) { cdb.close(); cb(false); return; }
                const tx = cdb.transaction('kv', 'readwrite');
                const store = tx.objectStore('kv');
                const gr = store.get('client_config');
                gr.onerror = () => { cdb.close(); cb(false); };
                gr.onsuccess = () => {
                    let c = {}; try { c = JSON.parse(gr.result || '{}'); } catch (e) {}
"#;

/// JS suffix: write `c` back as a JSON string and resolve once the transaction
/// commits.
const PATCH_CONFIG_SUFFIX: &str = r#"
                    store.put(JSON.stringify(c), 'client_config');
                    tx.oncomplete = () => { cdb.close(); cb(true); };
                    tx.onerror = () => { cdb.close(); cb(false); };
                };
            };
        };
    };
"#;

/// Run JS `mutate` against active-account client config `c`, with optional arguments from args, then persist to
/// IndexedDB. Panic if the write fails: otherwise offline preconditions can silently leave the app online and make
/// assertions invalid.
pub async fn patch_active_config(
    driver: &WebDriver,
    mutate: &str,
    args: Vec<serde_json::Value>,
) -> WebDriverResult<()> {
    let prefix = PATCH_CONFIG_PREFIX.replace("__RESOLVE_SEGMENT__", RESOLVE_SEGMENT_JS);
    let script = format!("{prefix}{mutate};{PATCH_CONFIG_SUFFIX}");
    let ok = driver
        .execute_async(&script, args)
        .await?
        .json()
        .as_bool()
        .unwrap_or(false);
    assert!(
        ok,
        "patch_active_config did not write: no active account, or the config DB \
         for the resolved segment has no `kv` store. If the storage namespace \
         changed, RESOLVE_SEGMENT_JS must be re-synced with \
         `halogen_webui_platform::namespace::segment_for`."
    );
    Ok(())
}

/// The active user's persisted client-config JSON (empty string if there's no
/// active user / no stored config — e.g. after a wipe).
pub async fn active_config_json(driver: &WebDriver) -> String {
    let script = r#"
        const cb = arguments[arguments.length - 1];
        const ar0 = indexedDB.open('halogen.accounts');
        ar0.onerror = () => cb('');
        ar0.onsuccess = () => {
            const adb = ar0.result;
            if (!adb.objectStoreNames.contains('registry')) { adb.close(); cb(''); return; }
            const gr0 = adb.transaction('registry', 'readonly').objectStore('registry').get('accounts');
            gr0.onerror = () => { adb.close(); cb(''); };
            gr0.onsuccess = () => {
                adb.close();
                const rawAccounts = gr0.result || '{}';
                let a = {}; try { a = JSON.parse(rawAccounts); } catch (e) {}
                __RESOLVE_SEGMENT__
                const cr = indexedDB.open('halogen.config.' + seg);
                cr.onerror = () => cb('');
                cr.onsuccess = () => {
                    const cdb = cr.result;
                    if (!cdb.objectStoreNames.contains('kv')) { cdb.close(); cb(''); return; }
                    const gr = cdb.transaction('kv', 'readonly').objectStore('kv').get('client_config');
                    gr.onerror = () => { cdb.close(); cb(''); };
                    gr.onsuccess = () => { cdb.close(); cb(gr.result || ''); };
                };
            };
        };
    "#
    .replace("__RESOLVE_SEGMENT__", RESOLVE_SEGMENT_JS);
    driver
        .execute_async(&script, Vec::new())
        .await
        .ok()
        .and_then(|ret| ret.json().as_str().map(str::to_string))
        .unwrap_or_default()
}

/// Drive the onboarding UI to an authenticated session: load the app (which redirects to the login page), then
/// connect to `base_url` with the given credentials — server URL + username + password on ONE form (native-app
/// parity). Leaves the browser on the authenticated Home route. Exercises the full stack: wasm UI → API client
/// → server (health, login, get_user). Panics with the page body on any step that doesn't appear.
pub async fn login_via_ui(driver: &WebDriver, base_url: &str, username: &str, password: &str) {
    driver.goto(base_url).await.expect("goto app");

    // The consolidated connect form: URL + credentials, one submit.
    assert!(
        wait_for_css(driver, "input[type='url']", Duration::from_secs(10)).await,
        "login URL field never appeared; body:\n{}",
        body_text(driver).await
    );
    fill(driver, "input[type='url']", base_url)
        .await
        .expect("type server url");
    fill(driver, "input[type='text']", username)
        .await
        .expect("type username");
    fill(driver, "input[type='password']", password)
        .await
        .expect("type password");
    click(driver, "button[type='submit']")
        .await
        .expect("click Connect");

    // Login + the post-auth redirect (`/` → `/queue`) settle asynchronously in
    // the SPA. Wait for the app chrome (the Queue nav link) before returning, so
    // callers act on an authenticated session instead of racing the
    // still-submitting login page (which RootGuard would bounce back to /login).
    assert!(
        wait_for_css(driver, "a[href='/queue']", Duration::from_secs(10)).await,
        "did not reach an authenticated view after login; body:\n{}",
        body_text(driver).await
    );
    screenshot::shot(driver, "01-authenticated").await;
}

// ── Upstream-feed fixtures (the only faked dependency) ───────────────────────

/// Read an RSS fixture from `data/tests/` (e.g. `sed_podcast.xml`). Panics with
/// the resolved path when the file is missing.
pub fn load_feed(name: &str) -> String {
    let path = format!("{}/../../data/tests/{}", env!("CARGO_MANIFEST_DIR"), name);
    std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("feed fixture not found: {path}"))
}

/// Mount a mock RSS feed, subscribe with the seeded admin token, and poll once for deterministic ingestion. Keep the
/// returned MockServer bound for the session so the upstream remains available.
pub async fn ingest_feed(
    app: &support::TestApp,
    token: &str,
    title: &str,
    feed_file: &str,
) -> MockServer {
    let upstream = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string(load_feed(feed_file)))
        .mount(&upstream)
        .await;
    app.seed_podcast(title, &upstream.uri()).await;
    let resp = reqwest::Client::new()
        .post(app.url("/api/v1/admin/poll"))
        .bearer_auth(token)
        .send()
        .await
        .expect("server poll");
    assert!(
        resp.status().is_success(),
        "server poll failed: {}",
        resp.status()
    );
    upstream
}

// ── List / device-store browser helpers ──────────────────────────────────────

/// The trimmed visible text of the first element matching `css` (empty string when
/// there's no match or it can't be read). Handy for "what's the top row?" checks.
pub async fn first_text(driver: &WebDriver, css: &str) -> String {
    match driver.query(By::Css(css)).first().await {
        Ok(el) => el.text().await.unwrap_or_default(),
        Err(_) => String::new(),
    }
}

/// Scroll the episode-list sentinel (`#episode-sentinel`) into view repeatedly,
/// driving the IntersectionObserver that grows the virtualization window / fetches
/// the next server page, until at least `target` rows match `css` or `timeout`
/// elapses. Returns the final row count.
pub async fn scroll_to_count(
    driver: &WebDriver,
    css: &str,
    target: usize,
    timeout: Duration,
) -> usize {
    let deadline = Instant::now() + timeout;
    loop {
        if count(driver, css).await >= target {
            break;
        }
        if let Ok(sentinel) = driver.find(By::Css("#episode-sentinel")).await {
            sentinel.scroll_into_view().await.ok();
        }
        if Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(400)).await;
    }
    count(driver, css).await
}

/// Return the cached subset of same-origin paths across all Cache API stores, or [] when unavailable. Inspect stored
/// responses directly: rendered images prove network availability, not offline survival. Cache names depend on the
/// frontend build fingerprint.
pub async fn sw_cached_paths(driver: &WebDriver, paths: &[&str]) -> Vec<String> {
    let cached = sw_cached_pathnames(driver).await;
    paths
        .iter()
        .filter(|p| cached.iter().any(|c| c == *p))
        .map(|p| p.to_string())
        .collect()
}

/// Enumerate stored request pathnames across all origin caches. cache.match creates different Accept headers and can
/// miss existing entries with Vary; stored keys avoid that mismatch. Sweep all caches because names include the
/// frontend build fingerprint.
pub async fn sw_cached_pathnames(driver: &WebDriver) -> Vec<String> {
    let script = r#"
        const cb = arguments[arguments.length - 1];
        (async () => {
            if (!self.caches) return cb([]);
            const names = await caches.keys();
            const out = [];
            for (const n of names) {
                const c = await caches.open(n);
                for (const req of await c.keys()) out.push(new URL(req.url).pathname);
            }
            cb(out);
        })().catch(() => cb([]));
    "#;
    driver
        .execute_async(script, Vec::new())
        .await
        .ok()
        .and_then(|ret| serde_json::from_value::<Vec<String>>(ret.json().clone()).ok())
        .unwrap_or_default()
}

/// The byte length of the cached response body for same-origin `pathname`, or
/// `-1` if the path isn't cached at all. Distinguishes a genuinely-cached
/// IMAGE (non-empty body) from an empty `204` that the art handler returns for
/// an art-less row — key presence alone can't (a 204 caches with `ok === true`).
pub async fn sw_cached_body_len(driver: &WebDriver, pathname: &str) -> i64 {
    let script = r#"
        const want = arguments[0];
        const cb = arguments[arguments.length - 1];
        (async () => {
            if (!self.caches) return cb(-1);
            for (const n of await caches.keys()) {
                const c = await caches.open(n);
                for (const req of await c.keys()) {
                    if (new URL(req.url).pathname === want) {
                        const resp = await c.match(req);
                        if (!resp) continue;
                        const buf = await resp.clone().arrayBuffer();
                        return cb(buf.byteLength);
                    }
                }
            }
            cb(-1);
        })().catch(() => cb(-1));
    "#;
    driver
        .execute_async(script, vec![serde_json::json!(pathname)])
        .await
        .ok()
        .and_then(|ret| ret.json().as_i64())
        .unwrap_or(-1)
}

/// Wait until the service worker controls the page — until then it sees no
/// `fetch` events and caches nothing, so any cache assertion would race the
/// registration rather than test it. `false` on timeout.
pub async fn wait_for_sw_control(driver: &WebDriver, timeout: Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        let script = r#"
            const cb = arguments[arguments.length - 1];
            if (!navigator.serviceWorker) return cb(false);
            cb(!!navigator.serviceWorker.controller);
        "#;
        let controlled = driver
            .execute_async(script, Vec::new())
            .await
            .ok()
            .and_then(|r| r.json().as_bool())
            .unwrap_or(false);
        if controlled {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    false
}

/// Seed tiny audio blobs into the active account's byte store via RESOLVE_SEGMENT_JS; refresh afterward to rehydrate
/// the worker. Panic on failed writes so subsequent assertions cannot silently test empty storage.
pub async fn idb_seed_audio(driver: &WebDriver, episode_ids: &[i32]) -> WebDriverResult<()> {
    let ids_json = serde_json::to_string(episode_ids).expect("serialize ids");
    let script = format!(
        r#"
        const cb = arguments[arguments.length - 1];
        const ids = {ids_json};
        const ar0 = indexedDB.open('halogen.accounts');
        ar0.onerror = () => cb('no registry');
        ar0.onsuccess = () => {{
            const adb = ar0.result;
            if (!adb.objectStoreNames.contains('registry')) {{ adb.close(); cb('no registry store'); return; }}
            const gr0 = adb.transaction('registry', 'readonly').objectStore('registry').get('accounts');
            gr0.onerror = () => {{ adb.close(); cb('registry read failed'); }};
            gr0.onsuccess = () => {{
                adb.close();
                const rawAccounts = gr0.result || '{{}}';
                let a = {{}}; try {{ a = JSON.parse(rawAccounts); }} catch (e) {{}}
                {RESOLVE_SEGMENT_JS}
                const req = indexedDB.open('halogen.media.' + seg);
                req.onerror = () => cb('open failed');
                req.onsuccess = () => {{
                    const db = req.result;
                    // The app creates `audio` at boot; if it's absent the segment is
                    // wrong (or the app never booted) — say so instead of throwing.
                    if (!db.objectStoreNames.contains('audio')) {{
                        db.close(); cb('no audio store in halogen.media.' + seg); return;
                    }}
                    try {{
                        const tx = db.transaction('audio', 'readwrite');
                        const store = tx.objectStore('audio');
                        for (const id of ids) {{
                            store.put(new Blob(['x'], {{ type: 'audio/mpeg' }}), id);
                        }}
                        tx.oncomplete = () => {{ db.close(); cb('ok'); }};
                        tx.onerror = () => {{ db.close(); cb('tx failed'); }};
                    }} catch (e) {{ db.close(); cb('threw ' + e); }}
                }};
            }};
        }};
    "#
    );
    let got = driver.execute_async(&script, Vec::new()).await?;
    let got = got.json().as_str().unwrap_or("<not a string>").to_string();
    assert_eq!(
        got, "ok",
        "idb_seed_audio failed to seed the device byte store. If the storage \
         namespace changed, RESOLVE_SEGMENT_JS must be re-synced with \
         `halogen_webui_platform::namespace::segment_for`."
    );
    Ok(())
}

/// Count blobs in halogen.media.{segment}, resolved via RESOLVE_SEGMENT_JS; opening bare halogen.media would create an
/// unrelated empty DB. Negative diagnostics: -1 count error, -2 JS threw, -3 DB open error, -4 script failure, -5
/// missing registry.
pub async fn idb_audio_count(driver: &WebDriver) -> i64 {
    let script = r#"
        const cb = arguments[arguments.length - 1];
        const ar0 = indexedDB.open('halogen.accounts');
        ar0.onerror = () => cb(-5);
        ar0.onsuccess = () => {
            const adb = ar0.result;
            if (!adb.objectStoreNames.contains('registry')) { adb.close(); cb(-5); return; }
            const gr0 = adb.transaction('registry', 'readonly').objectStore('registry').get('accounts');
            gr0.onerror = () => { adb.close(); cb(-5); };
            gr0.onsuccess = () => {
                adb.close();
                const rawAccounts = gr0.result || '{}';
                let a = {}; try { a = JSON.parse(rawAccounts); } catch (e) {}
                __RESOLVE_SEGMENT__
                const req = indexedDB.open('halogen.media.' + seg);
                req.onerror = () => cb(-3);
                req.onsuccess = () => {
                    try {
                        const db = req.result;
                        if (!db.objectStoreNames.contains('audio')) { db.close(); cb(0); return; }
                        const count = db.transaction('audio', 'readonly').objectStore('audio').count();
                        count.onsuccess = () => { db.close(); cb(count.result); };
                        count.onerror = () => { db.close(); cb(-1); };
                    } catch (e) { cb(-2); }
                };
            };
        };
    "#
    .replace("__RESOLVE_SEGMENT__", RESOLVE_SEGMENT_JS);
    driver
        .execute_async(&script, Vec::new())
        .await
        .ok()
        .and_then(|ret| ret.json().as_i64())
        .unwrap_or(-4)
}

/// Required CI gates fail when browser prerequisites are missing.
fn is_e2e_required() -> bool {
    std::env::var("HALOGEN_E2E_REQUIRED").is_ok_and(|value| matches!(value.as_str(), "1" | "true"))
}
