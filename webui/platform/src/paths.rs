//! Resolve native storage roots from absolute HALOGEN_DATA_DIR/HALOGEN_CONFIG_DIR overrides, then ProjectDirs with
//! XDG/Flatpak support, then HOME/.halogen or a loudly logged temp fallback. Never use the working directory. Cache
//! roots per process; tests isolate environment overrides through nextest.

use std::path::PathBuf;
use std::sync::OnceLock;

/// Root for app data: SQLite stores (`<root>/u{id}/halogen.db`), the shared
/// audio byte cache (`<root>/audio/`), and the device log.
pub fn data_root() -> PathBuf {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| resolve("HALOGEN_DATA_DIR", Kind::Data))
        .clone()
}

/// Root for config files: the per-user client config
/// (`<root>/u{id}/client.json`) and the device-global accounts registry.
pub fn config_root() -> PathBuf {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| resolve("HALOGEN_CONFIG_DIR", Kind::Config))
        .clone()
}

/// Root for the embedded (in-process) server's data, its SQLite DB, media downloads, overrides file, and secrets all
/// live under this one directory (`halogen-local-runtime` owns the layout). A single distinct name under [`data_root`]
/// so it can never collide with the UI's own claims (`u*`/`e*`/ `anon` namespaces, `audio/`, `webview/`, `device.log`),
/// and, because `data_root == config_root` on macOS, also not with `accounts.json`.
pub fn embedded_server_root() -> PathBuf {
    data_root().join("server")
}

enum Kind {
    Data,
    Config,
}

impl Kind {
    fn subdir(&self) -> &'static str {
        match self {
            Kind::Data => "data",
            Kind::Config => "config",
        }
    }
}

fn resolve(env_override: &str, kind: Kind) -> PathBuf {
    if let Some(dir) = std::env::var_os(env_override)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
    {
        return dir;
    }
    if let Some(dir) = platform_dir(&kind) {
        return dir;
    }
    if let Some(home) = std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
    {
        let dir = home.join(".halogen").join(kind.subdir());
        tracing::warn!(
            "platform app dirs unavailable; falling back to {}",
            dir.display()
        );
        return dir;
    }
    let dir = std::env::temp_dir().join("halogen").join(kind.subdir());
    tracing::warn!(
        "no home dir; falling back to TEMP storage at {} — data won't survive a reboot",
        dir.display()
    );
    dir
}

fn platform_dir(kind: &Kind) -> Option<PathBuf> {
    let dirs = directories::ProjectDirs::from("org", "fgsec", "halogen")?;
    Some(match kind {
        Kind::Data => dirs.data_dir().to_path_buf(),
        Kind::Config => dirs.config_dir().to_path_buf(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Each test runs in its own process under nextest, so the OnceLock memo
    // and env mutations can't leak between tests.

    #[test]
    fn env_override_wins() {
        let dir = std::env::temp_dir().join("halogen-paths-override-test");
        unsafe {
            std::env::set_var("HALOGEN_DATA_DIR", &dir);
        }
        assert_eq!(data_root(), dir);
    }

    #[test]
    fn relative_env_override_is_ignored() {
        unsafe {
            std::env::set_var("HALOGEN_DATA_DIR", "relative/never");
        }
        let root = data_root();
        assert!(root.is_absolute(), "got {}", root.display());
        assert_ne!(root, PathBuf::from("relative/never"));
    }

    #[test]
    fn roots_are_absolute_and_distinct() {
        let data = data_root();
        let config = config_root();
        assert!(data.is_absolute(), "data root: {}", data.display());
        assert!(config.is_absolute(), "config root: {}", config.display());
        // XDG layouts keep data and config apart; the single-base fallback
        // differs by subdir. macOS is the exception: `directories` puts BOTH
        // under Application Support/<id> — same dir, distinct filenames —
        // which is shipped desktop behavior, so equality is fine there.
        #[cfg(not(target_os = "macos"))]
        assert_ne!(data, config);
    }

    #[test]
    fn memoized_within_a_process() {
        let first = data_root();
        unsafe {
            std::env::set_var("HALOGEN_DATA_DIR", "/somewhere/else");
        }
        assert_eq!(data_root(), first);
    }
}
