//! The embedded server's on-disk layout: one root directory holding
//! everything, colliding with none of the UI's own claims under the app data
//! dir (`u*/`, `anon/`, `audio/`, `webview/`, `device.log` — and, because
//! `data_root == config_root` on macOS/iOS, also `accounts.json`).

use std::path::{Path, PathBuf};

/// Root of the embedded server's data. The UI passes
/// `ui-platform::paths::embedded_server_root()`; tests pass a temp dir.
///
/// ```text
/// <root>/
/// ├── halogen.db (+ -wal/-shm)   server SQLite (WAL profile)
/// ├── media/                     server downloads (+ media/art/ cache)
/// ├── overrides.toml             runtime config-overrides file
/// ├── secrets.json               token secret + admin credentials (0600)
/// └── lock                       single-instance advisory lock
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmbeddedDirs {
    root: PathBuf,
}

impl EmbeddedDirs {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Whether an embedded library already exists here (provisioned secrets
    /// are the marker — they're written before first serve). Drives the
    /// "reconnect vs create" copy on the confirmation page.
    pub fn library_exists(&self) -> bool {
        self.secrets_path().exists()
    }

    pub(crate) fn db_path(&self) -> PathBuf {
        self.root.join("halogen.db")
    }

    pub(crate) fn media_root(&self) -> PathBuf {
        self.root.join("media")
    }

    pub(crate) fn overrides_path(&self) -> PathBuf {
        self.root.join("overrides.toml")
    }

    pub(crate) fn secrets_path(&self) -> PathBuf {
        self.root.join("secrets.json")
    }

    pub(crate) fn lock_path(&self) -> PathBuf {
        self.root.join("lock")
    }
}
