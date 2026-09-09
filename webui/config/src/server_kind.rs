//! Which kind of server an account/config is bound to, and the composite account key built from it. These live here
//! (not in `halogen-webui-accounts`) because the config store's `*_for` variants need the key to compute a namespace
//! segment, and `ui-accounts` depends on this crate, not the other way around.

use serde::{Deserialize, Serialize};

/// Remote = a normal `https://…` server the user pointed the app at. Embedded = the native in-process server
/// (`halogen-local-runtime`), whose loopback URL is runtime state, never identity (see the load-time overlay in
/// `ClientConfigStore`). Local-only, never synced. Defaults to `Remote` so every pre-feature persisted config/registry
/// deserializes unchanged.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ServerKind {
    #[default]
    Remote,
    Embedded,
}

impl ServerKind {
    pub fn is_embedded(self) -> bool {
        matches!(self, ServerKind::Embedded)
    }
}

/// The composite identity of an account on this device. User ids are per-server (a remote server's admin and the
/// embedded admin share the same fixture-fixed id), so the bare id is ambiguous the moment both kinds exist, every
/// registry lookup and namespaced store keys on this pair instead. Segments: `u{id}` (remote) / `e{id}` (embedded).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AccountKey {
    pub kind: ServerKind,
    pub id: i32,
    /// Stable hash of the remote server's URL (0 for embedded — one embedded
    /// server per device). Distinguishes accounts on DIFFERENT remote servers
    /// that share a user id (both admin = 1), so they don't collapse onto one
    /// registry slot / storage namespace.
    pub server: u64,
}

impl AccountKey {
    pub fn remote(id: i32, server: u64) -> Self {
        Self {
            kind: ServerKind::Remote,
            id,
            server,
        }
    }

    pub fn embedded(id: i32) -> Self {
        Self {
            kind: ServerKind::Embedded,
            id,
            server: 0,
        }
    }

    pub fn is_embedded(self) -> bool {
        self.kind.is_embedded()
    }

    /// The storage namespace segment for this account (`u{id}-{server}` / `e{id}`).
    pub fn segment(self) -> String {
        halogen_webui_platform::namespace::segment_for(
            Some(self.id),
            self.kind.is_embedded(),
            self.server,
        )
    }
}

/// Deterministic hash of a server URL, normalized (trimmed + trailing-slash
/// stripped + lowercased) so trivial variations map together. Used as the
/// `server` component of a remote [`AccountKey`] (and thus its storage segment),
/// so it must be stable for a given URL. FNV-1a — fast and deterministic.
pub fn server_hash(url: &str) -> u64 {
    let normalized = url.trim().trim_end_matches('/').to_lowercase();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in normalized.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}
