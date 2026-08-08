//! Tracing init. One global subscriber: a stdout fmt layer always, an in-memory
//! ring layer always (backs `GET /admin/server-logs` when there's no log file —
//! stdout, once printed, is unrecoverable from inside the process), plus an
//! append-only file layer when `log_file` is set. `OnceLock`-guarded so a
//! restart's re-`init` is a no-op (the subscriber can only be set once).

use std::collections::VecDeque;
use std::fs;
use std::io;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::prelude::*;

use halogen_config::Config;

static _INIT_GUARD: OnceLock<()> = OnceLock::new();

/// Dependency targets the DEFAULT filter quiets. `sqlx` logs every statement it
/// executes at DEBUG, so a `log_level = debug` deployment buries its own app
/// logs under query spam. Capped rather than silenced — real pool/statement
/// failures still surface.
const QUIET_TARGETS: &[&str] = &["sqlx"];

/// How many formatted log lines the in-memory ring retains. At ~150 bytes/line
/// this is a few hundred KB — bounded, and enough recent context for the admin
/// server-logs page on a file-less deployment.
const RING_CAP: usize = 2_000;

/// The ring plus a partial-line accumulator: the fmt layer usually writes one
/// complete `\n`-terminated line per event, but nothing guarantees it, so bytes
/// are buffered until a newline lands.
static RING: Mutex<Option<(VecDeque<String>, String)>> = Mutex::new(None);

/// The last `n` in-memory log lines, oldest → newest. Empty before `init` (or
/// in test binaries that never install the subscriber).
pub fn recent_lines(n: usize) -> Vec<String> {
    let guard = RING.lock().unwrap_or_else(|p| p.into_inner());
    match guard.as_ref() {
        Some((lines, _)) => lines
            .iter()
            .skip(lines.len().saturating_sub(n))
            .cloned()
            .collect(),
        None => Vec::new(),
    }
}

/// Append formatted bytes to the ring, splitting complete lines off the
/// accumulator and evicting the oldest lines past [`RING_CAP`].
fn ring_push(buf: &[u8]) {
    let mut guard = RING.lock().unwrap_or_else(|p| p.into_inner());
    let (lines, partial) = guard.get_or_insert_with(|| (VecDeque::new(), String::new()));
    partial.push_str(&String::from_utf8_lossy(buf));
    while let Some(nl) = partial.find('\n') {
        let line: String = partial.drain(..=nl).collect();
        lines.push_back(line.trim_end().to_string());
        if lines.len() > RING_CAP {
            lines.pop_front();
        }
    }
}

/// `MakeWriter` feeding the in-memory ring — lets the ring reuse the exact fmt
/// layer output (same shape as the stdout/file lines) instead of hand-rolling a
/// second event formatter.
#[derive(Clone, Copy)]
struct RingWriter;

impl io::Write for RingWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        ring_push(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for RingWriter {
    type Writer = RingWriter;

    fn make_writer(&'a self) -> Self::Writer {
        RingWriter
    }
}

/// The filter directives every layer runs, as a `RUST_LOG` string.
///
/// `RUST_LOG` wins outright when the deployer sets it — their directives
/// replace ours wholesale, including `log_level`. Otherwise we ship
/// `<log_level>` plus a cap on [`QUIET_TARGETS`]. The cap never makes a target
/// LOUDER than the base level: at `log_level = error`, `sqlx=warn` would
/// promote query warnings above everything else, so the cap follows the base
/// down instead.
fn filter_directives(log_level: &str, rust_log: Option<&str>) -> String {
    if let Some(rust_log) = rust_log
        && !rust_log.trim().is_empty()
    {
        return rust_log.to_string();
    }
    let base = match log_level {
        level @ ("trace" | "debug" | "info" | "warn" | "error") => level,
        _ => "info",
    };
    let cap = if base == "error" { "error" } else { "warn" };
    let mut directives = String::from(base);
    for target in QUIET_TARGETS {
        directives.push_str(&format!(",{target}={cap}"));
    }
    directives
}

/// Install the global tracing subscriber from `cfg`. Idempotent (first call wins).
pub fn init(cfg: &Config) {
    _INIT_GUARD.get_or_init(|| {
        // `EnvFilter` isn't `Clone`, so each layer parses its own copy of the
        // one directive string.
        let directives =
            filter_directives(&cfg.log_level, std::env::var("RUST_LOG").ok().as_deref());
        let filter = || EnvFilter::new(&directives);

        let fmt_layer = tracing_subscriber::fmt::layer()
            .with_target(cfg.log_target)
            .with_file(cfg.log_file_name)
            .with_line_number(cfg.log_line_number)
            .with_writer(io::stdout)
            .with_filter(filter());

        // In-memory ring — always on, so `/admin/server-logs` has something to
        // serve on a file-less deployment. `with_ansi(false)`: the ring is read
        // back as plain text by the UI, never a terminal.
        let ring_layer = tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .with_target(cfg.log_target)
            .with_file(cfg.log_file_name)
            .with_line_number(cfg.log_line_number)
            .with_writer(RingWriter)
            .with_filter(filter());

        match &cfg.log_file {
            Some(log_path) => {
                fs::create_dir_all(
                    log_path
                        .parent()
                        .map(|p| p as &Path)
                        .unwrap_or_else(|| Path::new(".")),
                )
                .expect("create log dir");
                let file_app = fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(log_path)
                    .expect("open log file");
                let file_layer = tracing_subscriber::fmt::layer()
                    .with_writer(file_app)
                    .with_target(cfg.log_target)
                    .with_file(cfg.log_file_name)
                    .with_line_number(cfg.log_line_number)
                    .with_filter(filter());

                tracing_subscriber::registry()
                    .with(fmt_layer)
                    .with(ring_layer)
                    .with(file_layer)
                    .init();
            }
            None => {
                tracing_subscriber::registry()
                    .with(fmt_layer)
                    .with(ring_layer)
                    .init();
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shipped default: the configured level for everything, with sqlx
    /// capped so a `debug` deployment isn't drowned in per-statement spam.
    #[test]
    fn default_filter_caps_sqlx() {
        assert_eq!(filter_directives("debug", None), "debug,sqlx=warn");
        assert_eq!(filter_directives("info", None), "info,sqlx=warn");
        // Unparseable levels fall back to info rather than filtering everything out.
        assert_eq!(filter_directives("shout", None), "info,sqlx=warn");
    }

    /// The directives are only half the story: sqlx logs under `sqlx::query`,
    /// so the `sqlx=warn` cap works only because EnvFilter matches targets by
    /// PREFIX. Drives real events through a real filter to prove it.
    #[test]
    fn default_filter_suppresses_sqlx_query_events() {
        let subscriber = tracing_subscriber::registry().with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(RingWriter)
                .with_filter(EnvFilter::new(filter_directives("debug", None))),
        );
        tracing::subscriber::with_default(subscriber, || {
            tracing::debug!(target: "sqlx::query", "SELECT the whole episode table");
            tracing::warn!(target: "sqlx::query", "pool acquire timed out");
            tracing::debug!(target: "halogen_server", "an app debug line");
        });

        let logged = recent_lines(10).join("\n");
        assert!(
            !logged.contains("SELECT the whole episode table"),
            "sqlx DEBUG must be filtered out; got: {logged}"
        );
        assert!(
            logged.contains("pool acquire timed out"),
            "sqlx WARN must still surface; got: {logged}"
        );
        assert!(
            logged.contains("an app debug line"),
            "app DEBUG must be unaffected; got: {logged}"
        );
    }

    /// The cap must never make a target LOUDER than the base level.
    #[test]
    fn default_filter_never_raises_a_quiet_target() {
        assert_eq!(filter_directives("error", None), "error,sqlx=error");
    }

    /// A deployer-set `RUST_LOG` replaces our directives wholesale — including
    /// `log_level`, and including any sqlx cap.
    #[test]
    fn rust_log_overrides_the_default() {
        assert_eq!(
            filter_directives("info", Some("warn,sqlx=trace")),
            "warn,sqlx=trace",
            "the deployer asked for sqlx traces; give them sqlx traces"
        );
        // Set-but-blank is treated as unset — an empty RUST_LOG would otherwise
        // silence the server entirely.
        assert_eq!(filter_directives("debug", Some("")), "debug,sqlx=warn");
        assert_eq!(filter_directives("debug", Some("   ")), "debug,sqlx=warn");
    }

    // Exercises the ring directly (each nextest test runs in its own process,
    // so the global ring can't leak into other tests).
    #[test]
    fn ring_splits_lines_and_caps() {
        assert!(recent_lines(10).is_empty(), "empty before any push");

        // A partial write followed by its completion yields ONE line.
        ring_push(b"first ");
        assert!(recent_lines(10).is_empty(), "no newline yet");
        ring_push(b"line\nsecond line\n");
        assert_eq!(recent_lines(10), vec!["first line", "second line"]);

        // Tail semantics: ask for fewer than exist → newest kept.
        assert_eq!(recent_lines(1), vec!["second line"]);

        // Cap eviction drops the oldest.
        for i in 0..RING_CAP {
            ring_push(format!("line {i}\n").as_bytes());
        }
        let lines = recent_lines(RING_CAP + 10);
        assert_eq!(lines.len(), RING_CAP);
        assert_eq!(lines.last().unwrap(), &format!("line {}", RING_CAP - 1));
        assert!(!lines.contains(&"first line".to_string()), "oldest evicted");
    }
}
