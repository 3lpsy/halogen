//! The device-log core: the `Level`/`LogLine` types, the in-memory ring, the
//! `tracing` subscriber + `DeviceLogLayer` (`init`), and capture/snapshot/
//! search/export/download. Persistence is in the sibling `store` module; the
//! `tracing` macro re-exports stay at the crate root (see `lib.rs`).

use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use serde::{Deserialize, Serialize};
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer};
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::{JsCast, closure::Closure};

/// Max device-log lines kept in the ring buffer and persisted on disk/IndexedDB.
pub(crate) const CAP: usize = 5000;

/// Backstop cap for the not-yet-flushed `PENDING` queue. It normally drains every
/// 2s, but if the flush future stalls / panics / was never mounted it would grow
/// unbounded. Bound it to a few rings' worth and drop the oldest past that — the
/// ring stays the display source of truth, so a dropped pending line only means it
/// might miss persistence, never display.
const PENDING_CAP: usize = CAP * 4;

/// Device-log `EnvFilter`: our crate up to TRACE (the runtime [`Level`] atomic
/// narrows from there), noisy dependencies capped at WARN. Pinned — NOT driven by
/// `RUST_LOG` (see the dedicated `HALOGEN_DEVICE_LOG` env var below).
///
/// The `halogen_server`/`halogen_download`/`halogen_rss`/`halogen_polling`
/// entries exist for Embedded Server mode: the in-process server emits through
/// this app's subscriber (it never initializes its own), and its activity —
/// serving, feed syncs, download lifecycle — must be visible in the device-log
/// viewer, which is the embedded server's only log surface (`log_file` is None).
const DEVICE_FILTER_DEFAULT: &str = "info,halogen_ui=trace,halogen_server=info,halogen_download=info,halogen_rss=info,halogen_polling=info,halogen_embedded_server=info,hyper=warn,hyper_util=warn,sea_orm=warn,sqlx=warn,reqwest=warn,h2=warn,idb=warn";

/// Default console `EnvFilter`. Overridable via `RUST_LOG`.
///
/// `dioxus_signals=warn` is load-bearing for web perf: dioxus 0.7 puts a bare
/// `#[tracing::instrument]` (defaults to **INFO**) on `Signal::new_maybe_sync`
/// and `Memo::recompute` — the hottest path in the app. The `WASMLayer`'s
/// `on_enter`/`on_exit` emit a `performance.mark`+`measure` (a wasm→JS hop) for
/// every enabled span, so a bare `info` filter turned each signal/memo op into a
/// timing mark (thousands per render → seconds of main-thread blocking, and a
/// Lighthouse timeline drowned in `"new_maybe_sync"`/`"recompute"` measures).
/// Capping `dioxus_signals` at WARN keeps those internal spans off the timeline
/// (and the console) while preserving real warnings/errors.
const CONSOLE_FILTER_DEFAULT: &str = "info,hyper_util=warn,dioxus_signals=warn";

/// Severity of a captured log line. Ordered most-severe (`Error` = 0) to
/// least-severe (`Trace` = 4); a line is captured when `level as u8 <=` the
/// configured threshold, so threshold `Info` keeps Error/Warn/Info and drops
/// Debug/Trace.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
pub enum Level {
    Error = 0,
    Warn = 1,
    #[default]
    Info = 2,
    Debug = 3,
    Trace = 4,
}

impl Level {
    /// All variants, most-severe first, for the settings select.
    pub const ALL: [Level; 5] = [
        Level::Error,
        Level::Warn,
        Level::Info,
        Level::Debug,
        Level::Trace,
    ];

    /// Stable identifier (serde name) for `<select>` values.
    pub fn as_str(&self) -> &'static str {
        match self {
            Level::Error => "Error",
            Level::Warn => "Warn",
            Level::Info => "Info",
            Level::Debug => "Debug",
            Level::Trace => "Trace",
        }
    }

    pub fn from_str_or_default(s: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|l| l.as_str() == s)
            .unwrap_or_default()
    }

    /// Human label for the settings select.
    pub fn label(&self) -> &'static str {
        match self {
            Level::Error => "Error only",
            Level::Warn => "Warn and above",
            Level::Info => "Info and above (default)",
            Level::Debug => "Debug and above",
            Level::Trace => "Trace (everything)",
        }
    }

    /// daisyUI semantic text color for the viewer (theme-driven, not raw palette).
    pub fn color(&self) -> &'static str {
        match self {
            Level::Error => "text-error",
            Level::Warn => "text-warning",
            Level::Info => "text-base-content",
            Level::Debug => "text-info",
            Level::Trace => "text-muted",
        }
    }

    fn from_tracing(level: &tracing::Level) -> Self {
        match *level {
            tracing::Level::ERROR => Level::Error,
            tracing::Level::WARN => Level::Warn,
            tracing::Level::INFO => Level::Info,
            tracing::Level::DEBUG => Level::Debug,
            tracing::Level::TRACE => Level::Trace,
        }
    }
}

/// One captured log line.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogLine {
    /// Unix epoch milliseconds (UTC).
    pub ts_ms: i64,
    pub level: Level,
    /// Event target (`module_path!()` by default).
    pub target: String,
    pub msg: String,
}

impl LogLine {
    /// `HH:MM:SS.mmm` (UTC) for display.
    pub fn time_str(&self) -> String {
        chrono::DateTime::from_timestamp_millis(self.ts_ms)
            .map(|dt| dt.format("%H:%M:%S%.3f").to_string())
            .unwrap_or_else(|| self.ts_ms.to_string())
    }

    /// Full single-line render used for the viewer and the download/export.
    pub fn formatted(&self) -> String {
        format!(
            "{} {:<5} {}: {}",
            self.time_str(),
            self.level.as_str(),
            self.target,
            self.msg
        )
    }
}

// Runtime gate. Defaults are profile-based so device logging is live from the
// first line (before client config loads) and correct for dev/tests/prod even
// if `init()` is never called (e.g. in unit tests).
static ENABLED: AtomicBool = AtomicBool::new(cfg!(debug_assertions) || cfg!(test));
static LEVEL: AtomicU8 = AtomicU8::new(Level::Info as u8);

// The live ring (display source of truth) and the not-yet-persisted queue that
// the flush loop drains.
static RING: Mutex<VecDeque<LogLine>> = Mutex::new(VecDeque::new());
static PENDING: Mutex<Vec<LogLine>> = Mutex::new(Vec::new());

/// Initialise logging. Call once at app start, before `dioxus::launch`.
pub fn init() {
    // Seed the runtime gate with the profile default; ConfigProvider overrides
    // it with the persisted setting once the async config load completes.
    set_enabled(cfg!(debug_assertions) || cfg!(test));
    set_level(Level::Info);

    // The device ring is the in-app `/logs/device` viewer's data source, distinct
    // from the dev console. Pin it to its own filter so `RUST_LOG` (which is meant
    // to tune the console layer) can't silently widen or starve device capture —
    // that would defeat the deliberate console-vs-device two-filter split. A
    // dedicated `HALOGEN_DEVICE_LOG` env var still allows overriding it on its own.
    let device_filter = std::env::var("HALOGEN_DEVICE_LOG")
        .ok()
        .map(EnvFilter::new)
        .unwrap_or_else(|| EnvFilter::new(DEVICE_FILTER_DEFAULT));

    #[cfg(not(target_arch = "wasm32"))]
    let console = {
        let filter = EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| EnvFilter::new(CONSOLE_FILTER_DEFAULT));
        tracing_subscriber::fmt::layer().with_filter(filter)
    };

    #[cfg(target_arch = "wasm32")]
    let console = {
        // `WASMLayer` reaches for `web_sys::window()`/`performance`, which are `None`
        // inside a Web Worker — installing it there would panic on the first log. Gate
        // it on a `window` existing: the main thread gets the console layer, a Worker
        // context skips it (and relies on `DeviceLogLayer`, which is worker-safe:
        // `Date::now` + the in-memory ring). `Option<Layer>` is a no-op `Layer` when
        // `None`, so `.with(console)` below handles both cases — making `init()` safe
        // to call from a Worker (the worker normally uses `init_forwarding` instead).
        //
        // `WASMLayer` implements `Layer::enabled` (level <= its max). Added bare,
        // that acts as a GLOBAL filter — capping it at INFO would veto DEBUG/TRACE
        // for the whole subscriber, so they'd never reach `DeviceLogLayer` and the
        // device-log level control would do nothing. So set its internal max to
        // TRACE (no veto) and gate console verbosity with a PER-LAYER `EnvFilter`
        // (per-layer filters never veto other layers).
        web_sys::window().map(|_| {
            let filter = EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new(CONSOLE_FILTER_DEFAULT));
            let cfg = tracing_wasm::WASMLayerConfigBuilder::new()
                .set_max_level(tracing::Level::TRACE)
                .build();
            tracing_wasm::WASMLayer::new(cfg).with_filter(filter)
        })
    };

    // `try_init` (not `set_global_default`) so a double init in tests is a no-op
    // rather than a panic. Installed before launch → dioxus-logger stands down.
    let _ = tracing_subscriber::registry()
        .with(console)
        .with(DeviceLogLayer.with_filter(device_filter))
        .try_init();
}

/// Enable/disable device-log capture (live; called from settings).
pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}

pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// Set the capture threshold (live; called from settings).
pub fn set_level(level: Level) {
    LEVEL.store(level as u8, Ordering::Relaxed);
}

/// The current capture threshold. Mirrors the `LEVEL` atomic back into a [`Level`]
/// (the worker reads this to start capturing at the main thread's setting).
pub fn level() -> Level {
    match LEVEL.load(Ordering::Relaxed) {
        0 => Level::Error,
        1 => Level::Warn,
        3 => Level::Debug,
        4 => Level::Trace,
        _ => Level::Info,
    }
}

/// Push a line into the ring (display) and the pending queue (persistence),
/// honoring the cap. Shared by the tracing layer and the test-only [`capture`].
fn push(line: LogLine) {
    // Recover from a poisoned lock rather than dropping capture forever: the
    // protected data is a plain VecDeque/Vec of self-contained structs that is
    // always left consistent, so a panic elsewhere can't leave it half-updated.
    let mut ring = RING.lock().unwrap_or_else(|e| e.into_inner());
    if ring.len() >= CAP {
        ring.pop_front();
    }
    ring.push_back(line.clone());
    drop(ring);
    let mut pending = PENDING.lock().unwrap_or_else(|e| e.into_inner());
    // Drop-oldest backstop: if the flush loop stalled/panicked, keep PENDING bounded
    // rather than leaking unboundedly.
    if pending.len() >= PENDING_CAP {
        pending.remove(0);
    }
    pending.push(line);
}

/// Push a worker-originated (foreign) [`LogLine`] into the LOCAL ring + pending
/// queue. The sync Web Worker keeps no ring/store of its own — it forwards each
/// captured line to the main thread (see [`init_forwarding`]), which calls this so
/// the line lands in the SAME ring (live in the `/logs/device` viewer) and the SAME
/// `halogen.logs` store (persisted once, by the main thread). The line is taken
/// as-is (its `ts_ms`/level/target were already resolved in the worker).
pub fn ingest(line: LogLine) {
    push(line);
}

/// Directly record a line, applying the enable + level gate. Test-only: in the
/// app, capture happens in [`DeviceLogLayer`].
#[cfg(test)]
pub fn capture(level: Level, target: &str, args: std::fmt::Arguments) {
    if !enabled() || (level as u8) > LEVEL.load(Ordering::Relaxed) {
        return;
    }
    push(LogLine {
        ts_ms: now_ms(),
        level,
        target: target.to_string(),
        msg: format!("{args}"),
    });
}

/// Build a [`LogLine`] from a `tracing` event, applying the runtime enable + level
/// gates. Returns `None` when capture is disabled or the event is below the
/// threshold. Shared by [`DeviceLogLayer`] (which pushes the line into the local
/// ring) and [`ForwardLayer`] (which forwards it over the worker→main channel) so
/// both build lines identically.
fn event_to_logline(event: &tracing::Event<'_>) -> Option<LogLine> {
    if !enabled() {
        return None;
    }
    let meta = event.metadata();
    let level = Level::from_tracing(meta.level());
    if (level as u8) > LEVEL.load(Ordering::Relaxed) {
        return None;
    }
    let mut visitor = MessageVisitor::default();
    event.record(&mut visitor);
    Some(LogLine {
        ts_ms: now_ms(),
        level,
        target: meta.target().to_string(),
        msg: visitor.finish(),
    })
}

/// `tracing` layer that records admitted events into the device-log ring.
struct DeviceLogLayer;

impl<S: tracing::Subscriber> Layer<S> for DeviceLogLayer {
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        if let Some(line) = event_to_logline(event) {
            push(line);
        }
    }
}

/// `tracing` layer that hands each admitted event's [`LogLine`] to a callback
/// instead of recording it locally. The sync Web Worker installs this (via
/// [`init_forwarding`]) to ship its logs to the main thread, which owns the single
/// ring + `halogen.logs` store. No console layer, no local ring, no persistence.
struct ForwardLayer<F> {
    forward: F,
}

impl<S, F> Layer<S> for ForwardLayer<F>
where
    S: tracing::Subscriber,
    F: Fn(LogLine) + 'static,
{
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        if let Some(line) = event_to_logline(event) {
            (self.forward)(line);
        }
    }
}

/// Install a forwarding-only device-log subscriber: the ONLY layer builds a
/// [`LogLine`] on each admitted event (same `ENABLED`/`LEVEL` + device-filter gates
/// as [`init`]'s `DeviceLogLayer`) and calls `forward(line)`. No console layer, no
/// local ring, no persistence.
///
/// This is what the sync Web Worker calls *instead of* [`init`]: `init()` installs
/// the `WASMLayer` console (a main-thread thing — it needs `web_sys::window()`),
/// while a Worker has no ring/store of its own and instead forwards every line to
/// the main thread for the unified ring + `halogen.logs`.
///
/// Uses `try_init` so a double install is a no-op rather than a panic. Live toggling
/// of `ENABLED`/`LEVEL` after install is a documented follow-up — the worker is
/// seeded once via `ToWorker::Init`; there is no live-update channel today.
///
/// `forward` must be `Send + Sync` because `try_init` installs the subscriber as the
/// global default (`tracing` requires `Send + Sync + 'static` even on the
/// single-threaded wasm target). A worker that needs to touch `!Send` JS handles
/// (e.g. `postMessage`) should send the line through a `Send` channel and post from a
/// task, rather than capturing the handle in this closure.
pub fn init_forwarding(forward: impl Fn(LogLine) + Send + Sync + 'static) {
    let device_filter = std::env::var("HALOGEN_DEVICE_LOG")
        .ok()
        .map(EnvFilter::new)
        .unwrap_or_else(|| EnvFilter::new(DEVICE_FILTER_DEFAULT));
    let _ = tracing_subscriber::registry()
        .with(ForwardLayer { forward }.with_filter(device_filter))
        .try_init();
}

/// Renders an event's `message` field plus any structured fields into a single
/// line (`message key=value …`).
#[derive(Default)]
struct MessageVisitor {
    msg: String,
    fields: String,
}

impl MessageVisitor {
    fn finish(self) -> String {
        if self.fields.is_empty() {
            self.msg
        } else {
            format!("{}{}", self.msg, self.fields)
        }
    }
}

impl tracing::field::Visit for MessageVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        use std::fmt::Write;
        if field.name() == "message" {
            let _ = write!(self.msg, "{value:?}");
        } else {
            let _ = write!(self.fields, " {}={:?}", field.name(), value);
        }
    }
}

/// All captured lines, oldest → newest.
pub fn snapshot() -> Vec<LogLine> {
    RING.lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .cloned()
        .collect()
}

/// Lines whose message, target, or level matches `query` (case-insensitive),
/// oldest → newest.
pub fn search(query: &str) -> Vec<LogLine> {
    let q = query.to_lowercase();
    snapshot()
        .into_iter()
        .filter(|l| {
            l.msg.to_lowercase().contains(&q)
                || l.target.to_lowercase().contains(&q)
                || l.level.as_str().to_lowercase().contains(&q)
        })
        .collect()
}

/// Drop all captured lines from the ring (does not touch persisted storage;
/// callers also clear [`store`]). Also empties the not-yet-flushed `PENDING` queue
/// so a clear-without-drain can't be undone by the next 2s flush re-persisting the
/// lines we just cleared (e.g. `ui-cache-purge` clears without draining).
pub fn clear() {
    RING.lock().unwrap_or_else(|e| e.into_inner()).clear();
    PENDING.lock().unwrap_or_else(|e| e.into_inner()).clear();
}

/// Plain-text dump of all captured lines (newest last), for download/export.
pub fn export_text() -> String {
    snapshot()
        .iter()
        .map(LogLine::formatted)
        .collect::<Vec<_>>()
        .join("\n")
}

/// Take the not-yet-persisted lines, leaving the queue empty. Used by the flush
/// loop.
pub fn drain_pending() -> Vec<LogLine> {
    std::mem::take(&mut *PENDING.lock().unwrap_or_else(|e| e.into_inner()))
}

/// Seed the ring with lines loaded from persistent storage at startup. They are
/// inserted ahead of any lines already captured this session and NOT re-queued
/// for persistence.
pub fn ingest_persisted(lines: Vec<LogLine>) {
    if lines.is_empty() {
        return;
    }
    let mut ring = RING.lock().unwrap_or_else(|e| e.into_inner());
    for line in lines.into_iter().rev() {
        ring.push_front(line);
    }
    while ring.len() > CAP {
        ring.pop_front();
    }
}

/// Trigger a download of the full device log. On wasm this is a browser file
/// download; on native it writes a file and returns its path (for a toast).
pub fn download_logs(text: &str) -> Option<String> {
    download_text("halogen-device-logs.txt", text)
}

/// Trigger a download of `text` as `filename` — [`download_bytes`] with the
/// historical best-effort `Option` contract its callers expect (`None` covers
/// both "browser handled it" and "couldn't").
pub fn download_text(filename: &str, text: &str) -> Option<String> {
    download_bytes(filename, text.as_bytes(), "text/plain")
        .ok()
        .flatten()
}

/// Trigger a download of `bytes` as `filename` (e.g. the gzipped DB export).
/// On wasm: a browser download (Blob + synthetic anchor click) — `Ok(None)`,
/// the browser owns the outcome from there. On native: writes the file under
/// the app data directory — `Ok(Some(path))` for the caller's toast. `Err` is
/// a REAL failure (unwritable disk, missing DOM): callers must not report
/// success for it.
#[cfg(target_arch = "wasm32")]
pub fn download_bytes(filename: &str, bytes: &[u8], mime: &str) -> Result<Option<String>, String> {
    let fail = |what: &str| format!("Browser download failed ({what})");
    let win = web_sys::window().ok_or_else(|| fail("no window"))?;
    let doc = win.document().ok_or_else(|| fail("no document"))?;
    let array = js_sys::Uint8Array::from(bytes);
    let parts = js_sys::Array::of1(&array.into());
    let bag = web_sys::BlobPropertyBag::new();
    bag.set_type(mime);
    let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &bag)
        .map_err(|_| fail("blob"))?;
    let url = web_sys::Url::create_object_url_with_blob(&blob).map_err(|_| fail("object url"))?;
    let anchor = doc
        .create_element("a")
        .ok()
        .and_then(|el| el.dyn_into::<web_sys::HtmlAnchorElement>().ok())
        .ok_or_else(|| fail("anchor"))?;
    anchor.set_href(&url);
    anchor.set_download(filename);
    // Attach the anchor to the DOM before clicking and remove it after: some
    // browsers ignore a click on a detached anchor and yield an empty/aborted
    // download.
    let body = doc.body().ok_or_else(|| fail("no body"))?;
    let _ = body.append_child(&anchor);
    anchor.click();
    let _ = body.remove_child(&anchor);
    // Defer revoking the object URL to the next tick. Revoking synchronously right
    // after `click()` can abort the download before the browser has started reading
    // the blob. The one-shot closure is handed to the JS GC via `once_into_js`.
    let revoke = Closure::once_into_js(move || {
        let _ = web_sys::Url::revoke_object_url(&url);
    });
    let _ = win.set_timeout_with_callback_and_timeout_and_arguments_0(revoke.unchecked_ref(), 0);
    Ok(None)
}

#[cfg(not(target_arch = "wasm32"))]
pub fn download_bytes(filename: &str, bytes: &[u8], _mime: &str) -> Result<Option<String>, String> {
    let path = halogen_ui_platform::paths::data_root().join(filename);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(&path, bytes).map_err(|e| format!("Couldn't write {}: {e}", path.display()))?;
    Ok(Some(path.display().to_string()))
}

#[cfg(target_arch = "wasm32")]
fn now_ms() -> i64 {
    js_sys::Date::now() as i64
}

#[cfg(not(target_arch = "wasm32"))]
fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
#[cfg(test)]
mod tests {
    use super::*;
    use tracing_subscriber::layer::SubscriberExt;

    /// Reset all global state. nextest runs each test in its own process, so
    /// this is belt-and-suspenders for plain `cargo test` (shared process).
    fn reset(enabled: bool, level: Level) {
        clear();
        let _ = drain_pending();
        set_enabled(enabled);
        set_level(level);
    }

    fn line(level: Level, msg: &str) {
        capture(level, "test", format_args!("{msg}"));
    }

    #[test]
    fn disabled_captures_nothing() {
        reset(false, Level::Info);
        line(Level::Error, "nope");
        assert!(snapshot().is_empty());
    }

    #[test]
    fn level_threshold_filters() {
        reset(true, Level::Warn);
        line(Level::Error, "err");
        line(Level::Warn, "warn");
        line(Level::Info, "info"); // dropped: below threshold
        line(Level::Debug, "debug"); // dropped
        let msgs: Vec<_> = snapshot().into_iter().map(|l| l.msg).collect();
        assert_eq!(msgs, vec!["err", "warn"]);
    }

    #[test]
    fn ring_is_capped_dropping_oldest() {
        reset(true, Level::Trace);
        for i in 0..(CAP + 10) {
            line(Level::Info, &format!("m{i}"));
        }
        let snap = snapshot();
        assert_eq!(snap.len(), CAP);
        // Oldest 10 dropped, so the first kept line is m10.
        assert_eq!(snap.first().unwrap().msg, "m10");
        assert_eq!(snap.last().unwrap().msg, format!("m{}", CAP + 9));
    }

    #[test]
    fn search_matches_message_and_level() {
        reset(true, Level::Trace);
        line(Level::Info, "alpha");
        line(Level::Error, "beta");
        assert_eq!(search("alph").len(), 1);
        assert_eq!(search("error").len(), 1); // matches level name
        assert_eq!(search("zzz").len(), 0);
    }

    #[test]
    fn export_and_clear() {
        reset(true, Level::Trace);
        line(Level::Info, "hello");
        assert!(export_text().contains("hello"));
        clear();
        assert!(snapshot().is_empty());
    }

    #[test]
    fn drain_pending_yields_captured_then_empties() {
        reset(true, Level::Trace);
        line(Level::Info, "p1");
        line(Level::Info, "p2");
        let drained = drain_pending();
        assert_eq!(drained.len(), 2);
        assert!(drain_pending().is_empty());
    }

    #[test]
    fn ingest_persisted_prepends_older_lines() {
        reset(true, Level::Trace);
        line(Level::Info, "session");
        ingest_persisted(vec![LogLine {
            ts_ms: 1,
            level: Level::Info,
            target: "test".into(),
            msg: "persisted".into(),
        }]);
        let msgs: Vec<_> = snapshot().into_iter().map(|l| l.msg).collect();
        assert_eq!(msgs, vec!["persisted", "session"]);
    }

    /// End-to-end through the real `tracing` pipeline: the layer captures emitted
    /// events (message + structured fields) into the ring. Uses a scoped
    /// subscriber so it doesn't fight the global default.
    #[test]
    fn layer_captures_tracing_events() {
        reset(true, Level::Trace);
        let subscriber = tracing_subscriber::registry().with(DeviceLogLayer);
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!("via layer {}", 1);
            tracing::warn!(count = 5, "with fields");
        });
        let msgs: Vec<_> = snapshot().into_iter().map(|l| l.msg).collect();
        assert!(msgs.iter().any(|m| m == "via layer 1"), "{msgs:?}");
        assert!(
            msgs.iter()
                .any(|m| m.contains("with fields") && m.contains("count=5")),
            "{msgs:?}"
        );
    }

    /// The layer's runtime level gate (the Settings control) drops events below
    /// the threshold even when they pass the static filter.
    #[test]
    fn layer_respects_runtime_level() {
        reset(true, Level::Warn);
        let subscriber = tracing_subscriber::registry().with(DeviceLogLayer);
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!("info dropped");
            tracing::error!("error kept");
        });
        let msgs: Vec<_> = snapshot().into_iter().map(|l| l.msg).collect();
        assert_eq!(msgs, vec!["error kept"]);
    }
}
