use serde::{Deserialize, Serialize};

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

    pub(crate) fn from_tracing(level: &tracing::Level) -> Self {
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
