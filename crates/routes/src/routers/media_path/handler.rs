//! Confine DB-backed audio/art paths to media_root before serving. Only server download/cache pipelines set these
//! fields; canonical path checks also protect against legacy or hand-edited paths exposing arbitrary files.

use std::path::{Path, PathBuf};

/// Return the canonical candidate only when it remains inside canonical media_root. Resolve symlinks and ..; missing
/// paths, broken links, and unresolvable roots fail closed with None.
pub fn confined(media_root: &Path, candidate: &Path) -> Option<PathBuf> {
    let root = media_root.canonicalize().ok()?;
    let resolved = candidate.canonicalize().ok()?;
    resolved.starts_with(&root).then_some(resolved)
}

/// If a stored path is stale or outside media_root, try its basename directly under the current root, still enforcing
/// confinement. Flat episode filenames remain unambiguous when iOS moves the app container during updates.
pub fn confined_or_rebased(media_root: &Path, candidate: &Path) -> Option<PathBuf> {
    confined(media_root, candidate).or_else(|| {
        let name = candidate.file_name()?;
        confined(media_root, &media_root.join(name))
    })
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
