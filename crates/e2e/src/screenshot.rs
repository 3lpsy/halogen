//! Per-journey browser captures, the web half of `design/screenshots/`. One subdirectory per journey, one PNG
//! per named step — the same contract `just ios-screenshots` produces for the simulator. Capture is opt-in via
//! `HALOGEN_E2E_SCREENSHOT_DIR`, so an ordinary `just test-e2e` pays nothing: no directory, no round trip, no
//! PNG encode.

use std::path::PathBuf;
use std::sync::Mutex;

use thirtyfour::prelude::*;

/// Set to the output root to turn capture on; `just web-screenshots` and the
/// `test-e2e` workflow both point it at `design/screenshots/web`.
pub const SCREENSHOT_DIR_ENV: &str = "HALOGEN_E2E_SCREENSHOT_DIR";

/// The journey [`shot`] writes under. nextest runs one process per test, so
/// this is per-journey state; a test opening two sessions simply re-sets it.
static CURRENT: Mutex<String> = Mutex::new(String::new());

fn output_root() -> Option<PathBuf> {
    match std::env::var(SCREENSHOT_DIR_ENV) {
        Ok(dir) if !dir.is_empty() => Some(PathBuf::from(dir)),
        _ => None,
    }
}

/// Journey label to directory name: `"auth lifecycle"` → `auth-lifecycle`.
/// Runs of non-alphanumerics collapse to one `-`, and the ends are trimmed.
pub fn slug(what: &str) -> String {
    let mut out = String::with_capacity(what.len());
    for ch in what.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() {
        "journey".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Names the journey every later [`shot`] belongs to. `run_session` calls it.
pub(crate) fn set_journey(what: &str) {
    if output_root().is_none() {
        return;
    }
    if let Ok(mut current) = CURRENT.lock() {
        *current = slug(what);
    }
}

/// Write `<root>/<journey>/<step>.png`. Steps are named `NN-description` so the directory reads in order,
/// matching the iOS captures. Evidence, never an assertion: every failure here is logged and swallowed, so a
/// capture can never change a test's verdict.
pub async fn shot(driver: &WebDriver, step: &str) {
    let Some(root) = output_root() else {
        return;
    };
    let journey = match CURRENT.lock() {
        Ok(current) if !current.is_empty() => current.clone(),
        _ => "journey".to_string(),
    };
    let dir = root.join(journey);
    // thirtyfour's `screenshot` is a bare `fs::write` — it creates no parents.
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("screenshot: could not create {}: {e}", dir.display());
        return;
    }
    let path = dir.join(format!("{}.png", slug(step)));
    match tokio::time::timeout(std::time::Duration::from_secs(5), driver.screenshot(&path)).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => eprintln!("screenshot: {} not captured: {error}", path.display()),
        Err(_) => eprintln!("screenshot: {} timed out", path.display()),
    }
}
