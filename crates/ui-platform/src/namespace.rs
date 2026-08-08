//! Active-user storage namespace.
//!
//! Multi-account support partitions every per-user client store under a prefix
//! derived from the active user id (`halogen.u{id}.…` on web, an `u{id}/` subdir
//! on native). The active user is device-global state held in the account
//! registry (`halogen-ui-accounts`); this module is the tiny ambient mirror of
//! "which user's namespace is current" that the per-user stores
//! (`ClientConfigStore`, `ListViewStore` in `halogen-ui-config`, the local + media
//! stores) read when they build their keys/paths.
//!
//! Accounts on an **embedded** server (the native in-process server) get an
//! `e{id}` segment instead of `u{id}`: user ids are per-server (the embedded
//! admin is always the fixture's fixed id, and so is a remote server's), so
//! without the kind prefix a remote and an embedded account with equal ids
//! would silently share one namespace. The kind rides next to the id here as a
//! plain bool so this foundation crate stays free of the `ServerKind` type
//! (which lives up in `halogen-ui-config`).
//!
//! Why ambient (a process global) rather than a threaded parameter: the config /
//! list-view stores are called from ~6 scattered sites (navbar toggle, settings
//! effects, the login/connect form, the worker's auth-expired effect) inside spawned
//! async closures. Threading the active id through all of them would be invasive;
//! instead `AccountsProvider` sets this once per render — synchronously, before
//! the keyed data-layer subtree mounts — so every store read/write in that
//! subtree observes the right namespace. Single-threaded on web (wasm); on native
//! the app + each test process is effectively single-active-user, and nextest
//! isolates per process, so the global never races.

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};

/// Sentinel meaning "no active user" (pre-login / signed out). `i32::MIN` can't
/// collide with a real user id (server ids are positive).
const NONE: i32 = i32::MIN;

static ACTIVE_USER: AtomicI32 = AtomicI32::new(NONE);
/// Whether the active user belongs to the embedded (in-process) server.
/// Meaningless while `ACTIVE_USER` is `NONE`.
static ACTIVE_EMBEDDED: AtomicBool = AtomicBool::new(false);

/// Stable hash of the active REMOTE account's server URL (0 for embedded / anon).
/// User ids are per-server, so without this two accounts on DIFFERENT remote
/// servers that share an id (e.g. both admin = 1) would collapse onto one `u{id}`
/// namespace — sharing cached data and, worse, an outbox that then drains against
/// whichever server is active. Meaningless while `ACTIVE_USER` is `NONE`.
static ACTIVE_SERVER: AtomicU64 = AtomicU64::new(0);

/// Set the active user id (or `None` when signed out), whether that account lives
/// on the embedded server, and — for remote accounts — its server hash. Called by
/// `AccountsProvider` so subsequent store reads/writes hit the right namespace.
pub fn set_active(user_id: Option<i32>, embedded: bool, server: u64) {
    ACTIVE_USER.store(user_id.unwrap_or(NONE), Ordering::Relaxed);
    ACTIVE_EMBEDDED.store(embedded, Ordering::Relaxed);
    ACTIVE_SERVER.store(server, Ordering::Relaxed);
}

/// The active user id, or `None` when signed out.
pub fn active() -> Option<i32> {
    match ACTIVE_USER.load(Ordering::Relaxed) {
        NONE => None,
        id => Some(id),
    }
}

/// Whether the active account is on the embedded server (`false` when signed
/// out).
pub fn active_embedded() -> bool {
    active().is_some() && ACTIVE_EMBEDDED.load(Ordering::Relaxed)
}

/// The namespace segment for the active user: `"u{id}-{server}"` (remote) /
/// `"e{id}"` (embedded), or `"anon"` when there's no active user (a throwaway
/// namespace the idle, unauthenticated worker never writes). Used as the
/// localStorage / IndexedDB key infix and the native subdirectory.
pub fn segment() -> String {
    segment_for(
        active(),
        active_embedded(),
        ACTIVE_SERVER.load(Ordering::Relaxed),
    )
}

/// The namespace segment for a specific account. `embedded` selects the `e{id}`
/// prefix (one embedded server per device, so no server component); a remote
/// account is scoped by its `server` hash so two remote servers sharing a user id
/// don't share a namespace. Both are ignored for `None` (signed out → `"anon"`).
/// Used by the stores' `*_for` variants, which address a known account before the
/// ambient is set.
pub fn segment_for(user_id: Option<i32>, embedded: bool, server: u64) -> String {
    match user_id {
        Some(id) if embedded => format!("e{id}"),
        Some(id) => format!("u{id}-{server:016x}"),
        None => "anon".to_string(),
    }
}

/// Whether `name` is (or ever was) an account namespace segment — the reverse
/// of [`segment_for`], for consumers that enumerate storage (the cache-purge
/// failsafe walks directories / database names) rather than address a known
/// account. Lives HERE, next to the producer, so a format change can't leave a
/// purge matcher silently skipping the new shape again (the pre-server-hash
/// matcher only accepted `u{digits}` and left every `u{id}-{server}` account's
/// data — tokens included — on disk after a "delete everything").
///
/// Accepted: `anon`, embedded `e{id}`, current remote `u{id}-{16 hex}`, and the
/// legacy pre-server-hash remote `u{id}` (old installs still carry those dirs).
pub fn is_account_segment(name: &str) -> bool {
    if name == "anon" {
        return true;
    }
    let digits_then_hash = |rest: &str, hash: bool| -> bool {
        let (id, server) = match rest.split_once('-') {
            Some((id, server)) => (id, Some(server)),
            None => (rest, None),
        };
        if id.is_empty() || !id.chars().all(|c| c.is_ascii_digit()) {
            return false;
        }
        match server {
            // `{server:016x}` — fixed width, hex. Case-liberal on read.
            Some(s) if hash => s.len() == 16 && s.chars().all(|c| c.is_ascii_hexdigit()),
            Some(_) => false,
            None => true,
        }
    };
    match name.split_at_checked(1) {
        Some(("e", rest)) => digits_then_hash(rest, false),
        Some(("u", rest)) => digits_then_hash(rest, true),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segments_partition_by_kind_and_server() {
        // Remote is scoped by (id, server hash); embedded by id (one server).
        assert_eq!(segment_for(Some(7), false, 0xab), "u7-00000000000000ab");
        assert_eq!(segment_for(Some(7), true, 0), "e7");
        assert_eq!(segment_for(None, false, 0), "anon");
        assert_eq!(segment_for(None, true, 0), "anon");
        // Same id on DIFFERENT remote servers → DIFFERENT segments.
        assert_ne!(
            segment_for(Some(1), false, 0x11),
            segment_for(Some(1), false, 0x22),
        );
    }

    #[test]
    fn is_account_segment_matches_every_shape_segment_for_produces() {
        // The round trip that regressed pre-fix: every producible segment must
        // be recognized — a purge that misses one leaves that account's data
        // (tokens included) on disk after "delete everything".
        for seg in [
            segment_for(None, false, 0),
            segment_for(Some(1), true, 0),
            segment_for(Some(731), true, 0),
            segment_for(Some(1), false, 0),
            segment_for(Some(7), false, 0xab),
            segment_for(Some(42), false, u64::MAX),
        ] {
            assert!(is_account_segment(&seg), "unrecognized segment {seg}");
        }
        // Legacy pre-server-hash remote dirs still on old installs.
        assert!(is_account_segment("u3"));
    }

    #[test]
    fn is_account_segment_rejects_non_segments() {
        for name in [
            "",
            "u",
            "e",
            "u-",
            "server",
            "webview",
            "audio",
            "device.log",
            "ux1",
            "u1-",
            "u1-xyz",
            "u1-00000000000000ab-extra",
            "u1-00000000000000",   // 14 hex
            "e1-00000000000000ab", // embedded never carries a server hash
            "anonx",
        ] {
            assert!(!is_account_segment(name), "false positive on {name}");
        }
    }
}
