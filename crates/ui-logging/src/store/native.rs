//! Native device-log persistence: NDJSON at `<data root>/device.log`.
//! One JSON [`LogLine`](super::super::LogLine) per line, appended on flush and
//! pruned to [`CAP`](super::super::CAP) lines once the file grows past 2×CAP.

use std::io::Write;
use std::path::PathBuf;

use super::super::{CAP, LogLine};

/// Conservative lower bound on the serialized size of one NDJSON [`LogLine`] (the
/// four required keys + braces/quotes already exceed this even with empty strings).
/// Used only to gate the prune read: a file smaller than `2×CAP` of these can't
/// hold `2×CAP` lines, so it can't need pruning. Under-estimating is the safe
/// direction — it can only make the cheap gate trip a little early, never skip a
/// prune that's actually due.
const MIN_LINE_BYTES: u64 = 32;

fn log_path() -> Option<PathBuf> {
    // Option-shaped for the callers' early-return style; the platform paths
    // module itself always resolves (it falls back rather than failing).
    Some(halogen_ui_platform::paths::data_root().join("device.log"))
}

/// Load persisted lines (oldest → newest), capped to the last `CAP`.
pub async fn load_all() -> Vec<LogLine> {
    // Blocking fs runs on the flush path (every 2s); keep it off the async worker.
    tokio::task::spawn_blocking(|| {
        let Some(path) = log_path() else {
            return Vec::new();
        };
        let Ok(contents) = std::fs::read_to_string(&path) else {
            return Vec::new();
        };
        let mut lines: Vec<LogLine> = contents
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect();
        if lines.len() > CAP {
            lines.drain(0..lines.len() - CAP);
        }
        lines
    })
    .await
    .unwrap_or_default()
}

/// Append newly captured lines, then prune the file if it has grown past 2×CAP.
pub async fn append(new: &[LogLine]) {
    if new.is_empty() {
        return;
    }
    // Serialize up front so the moved closure owns plain strings, then run the
    // blocking fs work off the async worker (this is on the 2s flush path).
    let serialized: Vec<String> = new
        .iter()
        .filter_map(|line| serde_json::to_string(line).ok())
        .collect();
    let _ = tokio::task::spawn_blocking(move || {
        let Some(path) = log_path() else { return };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            for json in &serialized {
                let _ = writeln!(file, "{json}");
            }
        }
        // Amortized prune: rewrite keeping the last CAP lines once the file doubles.
        // Gate the expensive whole-file read on a cheap `metadata().len()` first —
        // a file below `2×CAP` minimum-length lines can't need pruning, so the
        // common 2s flush skips reading the file entirely.
        let big_enough = std::fs::metadata(&path)
            .map(|m| m.len() >= CAP as u64 * 2 * MIN_LINE_BYTES)
            .unwrap_or(false);
        if big_enough && let Ok(contents) = std::fs::read_to_string(&path) {
            let lines: Vec<&str> = contents.lines().collect();
            if lines.len() > CAP * 2 {
                let kept = lines[lines.len() - CAP..].join("\n");
                let _ = std::fs::write(&path, format!("{kept}\n"));
            }
        }
    })
    .await;
}

/// Delete the persisted log file.
pub async fn clear() {
    let _ = tokio::task::spawn_blocking(|| {
        if let Some(path) = log_path() {
            let _ = std::fs::remove_file(path);
        }
    })
    .await;
}

#[cfg(test)]
mod tests {
    use super::super::super::Level;
    use super::*;

    fn sandbox(tag: &str) {
        let base =
            std::env::temp_dir().join(format!("halogen-logstore-{tag}-{}", std::process::id()));
        let data = base.join("data");
        std::fs::remove_dir_all(&base).ok();
        std::fs::create_dir_all(&data).unwrap();
        unsafe {
            std::env::set_var("XDG_DATA_HOME", &data);
        }
    }

    fn line(msg: &str) -> LogLine {
        LogLine {
            ts_ms: 42,
            level: Level::Info,
            target: "test".into(),
            msg: msg.into(),
        }
    }

    #[tokio::test]
    async fn append_load_clear_roundtrip() {
        sandbox("roundtrip");
        assert!(load_all().await.is_empty());

        append(&[line("one"), line("two")]).await;
        append(&[line("three")]).await;

        let msgs: Vec<_> = load_all().await.into_iter().map(|l| l.msg).collect();
        assert_eq!(msgs, vec!["one", "two", "three"]);

        clear().await;
        assert!(load_all().await.is_empty());
    }

    #[tokio::test]
    async fn load_caps_to_last_cap_lines() {
        sandbox("cap");
        let batch: Vec<LogLine> = (0..CAP + 50).map(|i| line(&format!("m{i}"))).collect();
        append(&batch).await;
        let loaded = load_all().await;
        assert_eq!(loaded.len(), CAP);
        assert_eq!(loaded.first().unwrap().msg, "m50");
    }
}
