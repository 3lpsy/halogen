//! Shared helpers for browser end-to-end tests.
//!
//! The test pattern: build the wasm frontend (`just ui-build` → `dist/`), spawn
//! the **real** axum server serving that `dist/` as its `/` fallback (via
//! `halogen_integ::support`), then drive a headless browser at the
//! server's address with [`thirtyfour`]. As everywhere else, the only faked
//! dependency is upstream RSS (a `wiremock` server pointed to by `feed_url`).
//!
//! The tier is self-contained: [`ChromeDriver::start`] launches a `chromedriver`
//! child process on an ephemeral port and kills it on drop, so tests don't need
//! a pre-running driver. (Set `WEBDRIVER_URL` to reuse an external one instead.)

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

/// Connect a headless-Chrome session to a WebDriver `endpoint`.
///
/// Sets the browser binary explicitly (chromedriver otherwise looks only for
/// `google-chrome` and hangs on distros that ship `chromium-browser`), plus a
/// unique throwaway profile and the usual containerized-Chrome hardening flags.
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

/// Open a browser session, owning the chromedriver lifecycle.
///
/// Returns `None` (with a printed reason) only when the toolchain is genuinely
/// absent — `WEBDRIVER_URL` unset *and* no `chromedriver` binary — so tests can
/// skip on a bare machine. If chromedriver is present but the browser fails to
/// launch, that's a real error and panics.
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

/// Click `el`, scrolling it into view first and falling back to a synthetic
/// (JS) click when a native click can't land geometrically.
///
/// Two cases need the fallback:
/// - `ElementClickIntercepted`: daisyUI dropdowns stay open while focus is inside
///   them (Selenium clicks keep focus), so an open dropdown's expanded container
///   can sit over an adjacent control — a native click then lands on the overlay
///   rather than the intended button.
/// - `ElementNotInteractable`: the shared quick-context-menu panel is always
///   mounted and slides in via a 200ms `translate-x` transition. Its buttons
///   enter the DOM at transition *start*, while the panel is still translated off
///   the right edge — their in-view center point is outside the viewport, so a
///   native click is rejected as not-interactable until the slide settles.
///
/// A JS `.click()` dispatches straight to the element, bypassing the geometric
/// hit-test, which is what we want for these known-correct targets. (It still
/// won't fire on a `disabled` button, so genuinely-disabled controls aren't
/// silently actioned.)
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

/// Click the first element matching `css` (overlay-tolerant — see [`click_el`]).
///
/// Re-finds on staleness: a list/detail re-render between the find and the click
/// (e.g. a device-download progress tick replacing the Play button) can invalidate
/// the element handle (`StaleElementReference`). Re-query + retry a few times so a
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

/// Wait until exactly `target` elements match `css` and the count holds steady
/// for one extra poll (so a list mid-transition isn't sampled early). Returns the
/// last observed count — equal to `target` on success, otherwise whatever it was
/// when the timeout hit. Use for filter/search assertions where the set both
/// grows and shrinks.
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

/// Drain the browser console log (Chrome's `browser` buffer) and format it one
/// entry per line. This is where the wasm app's own `tracing` output lands (the
/// `WASMLayer` writes events to `console.*`), so it's the UI-side counterpart to
/// the server's stderr tracing — the two together explain most failures.
///
/// Console verbosity is the app's `CONSOLE_FILTER_DEFAULT` (INFO+); run with
/// `RUST_LOG=halogen_ui=debug` (rebuild `dist/`) for more. Returns an empty
/// string when the buffer is empty or the driver lacks the logging capability.
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

/// Keep only the signal from a raw browser-log entry: our wasm `tracing` (which
/// `WASMLayer` writes to `console.*` with `%c` colour formatting) and genuine
/// `SEVERE` errors. Everything else chromedriver surfaces — preload/credentials-
/// mode warnings, `<meta>` deprecation notices, autofocus chatter — is framework
/// noise and dropped. Tracing lines are unwrapped from their `%c…"color:…"`
/// wrapper into a plain `LEVEL target message`.
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

/// Run a browser test `body` to completion with uniform teardown: drive the body
/// with the session's `driver`, catch a panicking `assert!` (the common failure),
/// then [`finish_journey`] — dump the wasm console log, quit the driver, and
/// re-surface the outcome. This is the single closing every browser test uses, so
/// each one gets the on-failure log dump (not just the named journeys).
///
/// ```ignore
/// let Some((_guard, driver)) = browser_session().await else { return; };
/// run_session(driver, "onboarding journey", async |driver| {
///     login_via_ui(driver, &app.base_url, &admin.username, &admin.password).await;
///     /* assertions … */
///     Ok(())
/// })
/// .await;
/// ```
pub async fn run_session<F>(driver: WebDriver, what: &str, body: F)
where
    F: AsyncFnOnce(&WebDriver) -> WebDriverResult<()>,
{
    let outcome = std::panic::AssertUnwindSafe(body(&driver))
        .catch_unwind()
        .await;
    finish_journey(driver, outcome, what).await;
}

/// Close out a journey: dump the browser console log (the wasm app's tracing) to
/// stderr — which nextest shows on failure — then quit the driver and surface the
/// outcome. `outcome` is what [`FutureExt::catch_unwind`] yields around the test
/// body, so this runs whether the body returned `Ok`, returned a `WebDriverError`,
/// or panicked on an `assert!` (the common case) — none of which a plain post-body
/// statement would survive. Most tests reach this via [`run_session`]; call it
/// directly only when you need to interleave extra steps around teardown.
pub async fn finish_journey(
    driver: WebDriver,
    outcome: std::thread::Result<WebDriverResult<()>>,
    what: &str,
) {
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

// ── Per-user storage helpers ─────────────────────────────────────────────────
//
// Client config lives in IndexedDB (the localStorage backend was retired): the
// device-global account registry is `halogen.accounts` → store `registry` →
// record `accounts`, and each account's config is `halogen.config.{segment}` →
// store `kv` → record `client_config`. Each record is a JSON *string*, matching
// `halogen-ui-idb`'s JSON-string value encoding. These async helpers resolve the
// active account from the registry, so tests poke "the signed-in user's config"
// without hardcoding the namespace.

/// JS that resolves the active account's storage segment into `seg`, MIRRORING
/// `halogen_ui_platform::namespace::segment_for` — keep the two in step:
///
/// ```ignore
/// Some(id) if embedded => format!("e{id}"),
/// Some(id)             => format!("u{id}-{server:016x}"),
/// None                 => "anon",
/// ```
///
/// Expects the raw registry JSON in `rawAccounts` and its parsed form in `a`.
///
/// `active_server` is a **u64** and routinely exceeds `Number.MAX_SAFE_INTEGER`,
/// so `JSON.parse` silently rounds it and the hex comes out wrong — the digits are
/// re-read from the raw JSON text and widened with `BigInt` instead.
const RESOLVE_SEGMENT_JS: &str = r#"
    const _m = /"active_server"\s*:\s*(\d+)/.exec(rawAccounts);
    const _srv = (_m ? BigInt(_m[1]) : 0n).toString(16).padStart(16, '0');
    const seg = (a.active_user_id === null || a.active_user_id === undefined)
        ? 'anon'
        : (a.active_kind === 'Embedded'
            ? 'e' + a.active_user_id
            : 'u' + a.active_user_id + '-' + _srv);
"#;

/// JS prefix: open the registry, resolve the active account's config DB, open a
/// readwrite `kv` transaction, and parse the `client_config` record into `c`. Ends
/// right before the caller's mutate. (`cb`/`tx`/`store`/`cdb` are in scope after.)
///
/// Reports `false` rather than throwing; [`patch_active_config`] turns that into a
/// panic. A silent no-op here is worse than useless: `indexedDB.open` CREATES a
/// missing database, so a stale segment yields a fresh empty DB with no `kv`
/// store, the patch lands in a phantom the app never reads, and the test sails on
/// asserting against a still-online app.
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

/// Patch the active user's persisted client config: parse it as `c`, run `mutate`
/// (a JS statement with `c` in scope; it may reference `arguments[…]` from `args`),
/// then write it back to IndexedDB.
///
/// **Panics if the write didn't land.** Tests use this to establish a
/// precondition (e.g. "the server is now unreachable"); a patch that quietly does
/// nothing doesn't fail the test, it makes it VACUOUS — the app stays online and
/// every subsequent offline assertion is tested against an online app. That is
/// exactly what happened when the storage namespace gained a per-server suffix
/// and this helper kept opening the old path.
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
         `halogen_ui_platform::namespace::segment_for`."
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

/// Drive the onboarding UI to an authenticated session: load the app (which
/// redirects to the login page), then connect to `base_url` with the given
/// credentials — server URL + username + password on ONE form (native-app
/// parity). Leaves the browser on the authenticated Home route. Exercises the
/// full stack: wasm UI → API client → server (health, login, get_user).
/// Panics with the page body on any step that doesn't appear.
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
}

// ── Upstream-feed fixtures (the only faked dependency) ───────────────────────

/// Read an RSS fixture from `data/tests/` (e.g. `sed_podcast.xml`). Panics with
/// the resolved path when the file is missing.
pub fn load_feed(name: &str) -> String {
    let path = format!("{}/../../data/tests/{}", env!("CARGO_MANIFEST_DIR"), name);
    std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("feed fixture not found: {path}"))
}

/// Mount a `wiremock` RSS feed serving `feed_file`, subscribe the server to it,
/// and run one server-side `/api/v1/poll` so the feed is ingested deterministically
/// (the same proven path the journeys use — the UI subscribe form is an async
/// round-trip that doesn't reliably land in the test window).
///
/// Returns the live [`MockServer`]; bind it (`let _feed = …`) so upstream stays up
/// for the rest of the session. `token` is the seeded admin's bearer token.
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

/// Rows in the `halogen.media` IndexedDB `audio` store (the device byte store) —
/// the ground truth for "is this episode really downloaded to the device?".
/// Negative values encode failures so assertion messages stay useful:
/// `0` no store yet, `-1` count error, `-2` JS threw, `-3` open error, `-4` no/bad
/// return.
/// Which same-origin paths the service worker has actually stored, out of
/// `paths`. Returns the cached subset (order not guaranteed).
///
/// Reads the Cache API directly rather than inferring from rendered `<img>`s: an
/// `<img>` proves the *network* served it, which is exactly the thing that is
/// true right up until you go offline. Only the cache contents answer "will this
/// survive losing the server".
///
/// Searches every cache the origin owns (the SW pins its cache name to the build's
/// wasm fingerprint — see `_sync-dist` — so the name is not knowable from a test).
/// `[]` when the Cache API is unavailable or nothing matched.
pub async fn sw_cached_paths(driver: &WebDriver, paths: &[&str]) -> Vec<String> {
    let cached = sw_cached_pathnames(driver).await;
    paths
        .iter()
        .filter(|p| cached.iter().any(|c| c == *p))
        .map(|p| p.to_string())
        .collect()
}

/// Every same-origin pathname the service worker currently has stored, across all
/// caches this origin owns.
///
/// Enumerates `cache.keys()` rather than calling `cache.match(url)`: `match`
/// synthesizes a fresh `Request` whose `Accept`/`Accept-Encoding` won't match the
/// original (an `<img>` sends its own), so any `Vary` on the cached response makes
/// a genuinely-cached entry look absent. Comparing stored keys sidesteps content
/// negotiation entirely, which is what "is this path cached" actually means.
///
/// The SW pins its cache name to the build's wasm fingerprint (see `_sync-dist`),
/// so the name isn't knowable from a test — hence sweeping every cache.
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

/// Seed a tiny audio blob for each of `episode_ids` into the ACTIVE account's
/// device byte store, as a real device download would leave behind. Callers
/// usually `driver.refresh()` afterwards so the worker re-hydrates from them.
///
/// Namespaced via [`RESOLVE_SEGMENT_JS`] — see [`idb_audio_count`] for why a bare
/// `halogen.media` is the wrong database. Panics if the write didn't land: seeding
/// is a precondition, and silently seeding nothing turns the assertions that
/// follow into a test of the empty case.
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
         `halogen_ui_platform::namespace::segment_for`."
    );
    Ok(())
}

/// How many audio blobs the ACTIVE account has stored on device.
///
/// Namespaced like the config DB (`halogen.media.{segment}`) — a bare
/// `halogen.media` doesn't exist, and `indexedDB.open` would just conjure an empty
/// one, report 0 blobs forever, and make every "downloaded to device" assertion
/// pass or fail for the wrong reason. See [`RESOLVE_SEGMENT_JS`].
///
/// Negative returns are diagnostics, never counts: `-1` count error, `-2` threw,
/// `-3` media DB wouldn't open, `-4` the script itself failed, `-5` no registry.
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
