//! Shared test-only support for the crate-internal `#[cfg(test)]` suites across
//! the workspace: a self-cleaning temp [`TestRoot`] (DB/media scratch dir) and the
//! [`load_fixture`] raw-text loader (reads to a `String`) for `data/tests/*`. Gated behind the
//! `test-support` feature so it never ships in a normal build.

use std::env;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

fn test_root_dir() -> PathBuf {
    env::var("TEST_ROOT")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| env::temp_dir().join("halogen_test"))
}

pub struct TestRoot {
    path: PathBuf,
    success: bool,
}

impl TestRoot {
    pub fn new(suite: &str) -> Self {
        let base = test_root_dir();
        let suite_dir = base.join(suite);

        Self::sweep_failed(&suite_dir);

        // Namespace by PID *and* counter: nextest runs each test in its own
        // process, so the per-process counter alone resets to 0 in every process
        // and all tests would collide on `<suite>/0/halogen.db`. The PID keeps
        // concurrent test processes isolated; the counter separates multiple
        // TestRoots within one process.
        let counter = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let path = suite_dir.join(format!("{}_{counter}", std::process::id()));

        // Remove stale halogen.db from previous runs so migrations always
        // start from a blank slate (SQLite creates empty files on connect).
        let _ = fs::remove_file(path.join("halogen.db"));

        fs::create_dir_all(&path).expect("Failed to create test root directory");

        Self {
            path,
            success: false,
        }
    }

    pub fn mark_success(&mut self) {
        self.success = true;
    }

    #[allow(dead_code)]
    pub fn join(&self, part: &str) -> PathBuf {
        self.path.join(part)
    }

    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    fn sweep_failed(suite_dir: &PathBuf) {
        let _ = fs::create_dir_all(suite_dir);
        if let Ok(entries) = fs::read_dir(suite_dir) {
            for entry in entries.filter_map(|e| e.ok()) {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                if name_str.ends_with("_failed") {
                    let path = entry.path();
                    if path.is_dir() {
                        let _ = fs::remove_dir_all(&path);
                    } else {
                        let _ = fs::remove_file(&path);
                    }
                }
            }
        }
    }
}

impl Drop for TestRoot {
    fn drop(&mut self) {
        if self.success {
            if self.path.is_dir() {
                let _ = fs::remove_dir_all(&self.path);
            }
        } else if env::var("TEST_ROOT_DEBUG").is_ok() {
            let mut failed = self.path.clone();
            let mut name = failed
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            if !name.ends_with("_failed") {
                name.push_str("_failed");
            }
            failed.set_file_name(&name);

            let mut counter = 0u32;
            while failed.exists() {
                counter += 1;
                failed.set_file_name(format!("{name}_failed_{counter}"));
            }

            let _ = fs::rename(&self.path, &failed);
        }
    }
}

#[allow(dead_code)]
pub fn with_test_root(suite: &str, f: impl FnOnce(&TestRoot) -> io::Result<()>) {
    let mut root = TestRoot::new(suite);
    if f(&root).is_ok() {
        root.mark_success();
    }
}

/// Load a raw fixture from `data/tests/<name>` as a string, panicking with the
/// resolved path if it is missing. The path is relative to this crate's manifest
/// dir (`crates/fixture`), so `../../data/tests` resolves to the repo root.
pub fn load_fixture(name: &str) -> String {
    let path = format!("{}/../../data/tests/{}", env!("CARGO_MANIFEST_DIR"), name);
    fs::read_to_string(&path).unwrap_or_else(|_| {
        panic!("test fixture not found: {path}. Run the download script first.")
    })
}
