//! Build script — force a recompile (and thus a fresh rust-embed bake of the
//! frontend) whenever `dist/` changes.
//!
//! With the `embed-frontend` feature + `debug-embed`, `dist/` is baked into the
//! binary at COMPILE time. Cargo fingerprints source files, not the directory a
//! proc-macro happens to read, so a `just ui-build` that rewrites `dist/` (e.g. a
//! new wasm bundle) does NOT by itself invalidate the cached server binary —
//! cargo reuses the old build with STALE embedded assets. That bit the E2E
//! suite: an old wasm kept calling the (now-authed) `/status` for its
//! login page's health probe instead of the public `/healthz`, so the probe 401'd
//! and the login form never appeared.
//!
//! Emitting `rerun-if-changed` for every file under `dist/` ties the server's
//! fingerprint to the embedded assets, so any `ui-build` forces a re-embed.

use std::path::Path;

fn main() {
    // Only the embedded-frontend build bakes `dist/` in; otherwise there's
    // nothing to keep fresh. Cargo exports `CARGO_FEATURE_<NAME>` for active
    // features during build-script runs.
    if std::env::var_os("CARGO_FEATURE_EMBED_FRONTEND").is_none() {
        return;
    }

    // `dist/` lives at the workspace root (two levels up from crates/server).
    let dist = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../dist");
    // Track the directory itself (covers add/remove) and every file within.
    println!("cargo:rerun-if-changed={}", dist.display());
    track_dir(&dist);
}

/// Recursively emit `rerun-if-changed` for every entry under `dir`. Silently
/// no-ops if `dist/` is absent (a non-embed build, or before the first
/// `ui-build`) — the feature-gated embed just falls back to its own error path.
fn track_dir(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        println!("cargo:rerun-if-changed={}", path.display());
        if path.is_dir() {
            track_dir(&path);
        }
    }
}
