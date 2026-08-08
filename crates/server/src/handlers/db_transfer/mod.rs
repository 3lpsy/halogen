pub mod export;
pub mod import;

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// A unique scratch directory for one export/import operation
/// (`<tmp>/halogen-db-<kind>-<pid>-<nanos>-<counter>`). One shared recipe so
/// collision-avoidance and future hardening can't drift between the two
/// handlers; callers own cleanup (`fs::remove_dir_all`) on every exit path.
pub(crate) fn staging_dir(kind: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!(
        "halogen-db-{kind}-{}-{}-{}",
        std::process::id(),
        nanos,
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ))
}
