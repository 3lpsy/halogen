//! Filesystem confinement for the media file-serving endpoints (audio + art).
//!
//! The audio/art handlers stream a path read from a DB column
//! (`content_file_path` / `art_file_path`). Those columns are server-managed:
//! only the download and art-cache pipelines write them, always under
//! `media_root`, and the episode API DTOs no longer accept them from clients.
//! This module is the defense-in-depth backstop — before serving, resolve the
//! path and confirm it stays inside `media_root`, so a stray, legacy, or
//! hand-edited absolute path can never turn a media endpoint into an
//! arbitrary-file read.

use std::path::{Path, PathBuf};

/// The canonical form of `candidate` iff it resolves to a location inside
/// `media_root` (itself canonicalized). Returns `None` — meaning "refuse to
/// serve" — when the path escapes the root, is a broken symlink, or does not
/// exist. `canonicalize` resolves `..` and symlinks on both sides, so neither a
/// `../` sequence nor a symlink pointing out of the tree can defeat the check.
/// Fails closed: an unresolvable `media_root` (or candidate) yields `None`.
pub fn confined(media_root: &Path, candidate: &Path) -> Option<PathBuf> {
    let root = media_root.canonicalize().ok()?;
    let resolved = candidate.canonicalize().ok()?;
    resolved.starts_with(&root).then_some(resolved)
}

/// [`confined`], with a self-heal for RELOCATED media roots: when the stored
/// absolute path no longer resolves (or escapes the root) but a file of the
/// same name exists directly under `media_root`, serve that instead. The
/// download pipeline writes audio flat as `media_root/<episode_id>.<ext>`, so
/// the basename is unambiguous. This is what keeps stored downloads playable
/// on iOS, where the OS moves the app container (new UUID path) on every app
/// update/reinstall — the DB's absolute `content_file_path` goes stale while
/// the file itself was carried along. Confinement still applies to the
/// rebased path, so this can never serve outside the root.
pub fn confined_or_rebased(media_root: &Path, candidate: &Path) -> Option<PathBuf> {
    confined(media_root, candidate).or_else(|| {
        let name = candidate.file_name()?;
        confined(media_root, &media_root.join(name))
    })
}

#[cfg(test)]
mod tests {
    use super::confined;
    use halogen_fixture::test_support::TestRoot;
    use std::fs;

    #[test]
    fn accepts_files_inside_the_root() {
        let mut root = TestRoot::new("media_path_inside");
        let media = root.path().join("media");
        fs::create_dir_all(media.join("art")).unwrap();

        let audio = media.join("42.mp3");
        fs::write(&audio, b"x").unwrap();
        assert!(
            confined(&media, &audio).is_some(),
            "audio under root serves"
        );

        let art = media.join("art").join("p.png");
        fs::write(&art, b"x").unwrap();
        assert!(
            confined(&media, &art).is_some(),
            "art subdir is inside root"
        );

        root.mark_success();
    }

    #[test]
    fn rebases_a_stale_absolute_path_onto_the_root() {
        let mut root = TestRoot::new("media_path_rebase");
        let media = root.path().join("media");
        fs::create_dir_all(&media).unwrap();
        fs::write(media.join("7.mp3"), b"x").unwrap();

        // The path a previous app-container location baked into the DB.
        let stale = root
            .path()
            .join("old-container")
            .join("media")
            .join("7.mp3");
        assert!(
            super::confined(&media, &stale).is_none(),
            "stale absolute path must not confine"
        );
        let rebased = super::confined_or_rebased(&media, &stale).expect("rebase serves");
        assert!(rebased.ends_with("7.mp3"));

        root.mark_success();
    }

    #[test]
    fn rejects_a_file_outside_the_root() {
        let mut root = TestRoot::new("media_path_outside");
        let media = root.path().join("media");
        fs::create_dir_all(&media).unwrap();
        // A real, readable file that lives outside media_root.
        let outside = root.path().join("secret.txt");
        fs::write(&outside, b"secret").unwrap();
        assert!(
            confined(&media, &outside).is_none(),
            "an existing file outside media_root must be refused"
        );
        root.mark_success();
    }

    #[test]
    fn rejects_a_traversal_escape() {
        let mut root = TestRoot::new("media_path_traversal");
        let media = root.path().join("media");
        fs::create_dir_all(&media).unwrap();
        let outside = root.path().join("secret.txt");
        fs::write(&outside, b"secret").unwrap();
        // `<media_root>/../secret.txt` resolves outside the root.
        let sneaky = media.join("..").join("secret.txt");
        assert!(
            confined(&media, &sneaky).is_none(),
            "a `../` escape must be refused"
        );
        root.mark_success();
    }
}
