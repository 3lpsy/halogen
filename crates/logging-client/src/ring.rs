use crate::{Level, LogLine};
use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

/// Maximum captured and persisted log lines.
pub const CAP: usize = 5000;
const PENDING_CAP: usize = CAP * 4;

// Runtime gate. Defaults are profile-based so device logging is live from the
// first line (before client config loads) and correct for dev/tests/prod even
// if `init()` is never called (e.g. in unit tests).
static ENABLED: AtomicBool = AtomicBool::new(cfg!(debug_assertions) || cfg!(test));
pub(crate) static LEVEL: AtomicU8 = AtomicU8::new(Level::Info as u8);

// The live ring (display source of truth) and the not-yet-persisted queue that
// the flush loop drains.
static RING: Mutex<VecDeque<LogLine>> = Mutex::new(VecDeque::new());
static PENDING: Mutex<Vec<LogLine>> = Mutex::new(Vec::new());

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
pub(crate) fn push(line: LogLine) {
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

/// Add an already-filtered worker log to the host ring and persistence queue.
pub fn ingest(line: LogLine) {
    push(line);
}

/// Directly record a line, applying the enable + level gate. Test-only: in the
/// app, capture happens in the tracing layer.
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

/// Clear captured and pending lines; callers clear platform persistence separately.
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

#[cfg(target_arch = "wasm32")]
pub(crate) fn now_ms() -> i64 {
    js_sys::Date::now() as i64
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
