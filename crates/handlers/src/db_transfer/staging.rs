use std::fs::{DirBuilder, File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Create a private, exclusive scratch directory. Callers remove it after use.
pub(crate) fn staging_dir(kind: &str) -> io::Result<PathBuf> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let path = std::env::temp_dir().join(format!(
        "halogen-db-{kind}-{}-{}-{}",
        std::process::id(),
        nanos,
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let mut builder = DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(&path)?;
    Ok(path)
}

/// Refuse existing files and symlinks, with private permissions from creation.
pub(crate) fn staging_file(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

pub const DB_IMPORT_MAX_BYTES: usize = 256 * 1024 * 1024;
pub const DB_IMPORT_MAX_DECOMPRESSED_BYTES: usize = 4 * DB_IMPORT_MAX_BYTES;

#[cfg(test)]
#[path = "staging/tests.rs"]
mod tests;
