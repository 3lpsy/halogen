use crate::ring::capture;
use crate::subscriber::DeviceLogLayer;
use crate::*;
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
