//! Mirror the active account namespace for config, view, metadata, and media stores. AccountsProvider sets it before
//! mounting the keyed user subtree; local/remote server identity prevents equal IDs from sharing storage. Web is
//! single-threaded and native uses one active account per process.

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};

/// Sentinel meaning "no active user" (pre-login / signed out). `i32::MIN` can't
/// collide with a real user id (server ids are positive).
const NONE: i32 = i32::MIN;

static ACTIVE_USER: AtomicI32 = AtomicI32::new(NONE);
/// Whether the active user belongs to the embedded (in-process) server.
/// Meaningless while `ACTIVE_USER` is `NONE`.
static ACTIVE_EMBEDDED: AtomicBool = AtomicBool::new(false);

/// Stable hash of the active REMOTE account's server URL (0 for embedded / anon). User ids are per-server, so without
/// this two accounts on DIFFERENT remote servers that share an id (e.g. both admin = 1) would collapse onto one `u{id}`
/// namespace, sharing cached data and, worse, an outbox that then drains against whichever server is active.
/// Meaningless while `ACTIVE_USER` is `NONE`.
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

/// The namespace segment for a specific account. `embedded` selects the `e{id}` prefix (one embedded server per device,
/// so no server component); a remote account is scoped by its `server` hash so two remote servers sharing a user id
/// don't share a namespace. Both are ignored for `None` (signed out → `"anon"`). Used by the stores' `*_for` variants,
/// which address a known account before the ambient is set.
pub fn segment_for(user_id: Option<i32>, embedded: bool, server: u64) -> String {
    match user_id {
        Some(id) if embedded => format!("e{id}"),
        Some(id) => format!("u{id}-{server:016x}"),
        None => "anon".to_string(),
    }
}

/// Recognize current and legacy account segments for storage enumeration/purging: anon, e{id}, u{id}-{16 hex}, and old
/// u{id}. Keep this matcher beside the producer so namespace changes cannot leave tokens/data behind after a full wipe.
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
