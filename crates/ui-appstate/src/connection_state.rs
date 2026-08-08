//! Connectivity + sync-activity state — the worker-owned reactive slice.
//!
//! Its own root signal so the connectivity WebSocket's ~4×/sec latency pongs (and
//! the Syncing/Saving activity churn) only re-render connection consumers — the
//! navbar status dot, offline-gated controls — not every component that reads the
//! main app state. The sync worker is the only writer; the UI reads it reactively
//! (see `halogen_ui_state::hooks::{use_connection, use_sync_status, use_is_offline,
//! use_connection_health}`).

use chrono::{DateTime, Utc};

use crate::state::{ConnectionHealth, SyncStatus};

/// Connectivity + sync-progress state; the worker is the sole writer.
///
/// `PartialEq` is load-bearing (same reason as `EpisodeState`): the memo slices
/// (`use_sync_status`, `use_connection_health`) gate re-renders on it. The
/// `connectionstate_is_partialeq` canary guards the derive.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct ConnectionState {
    /// Connectivity and sync progress status. Drives the download/sync gating
    /// (`is_offline`) and the "Syncing…/Saving…" activity it encodes; the navbar's
    /// status dot reads [`ConnectionState::connection`] instead.
    pub sync_status: SyncStatus,
    /// Live connection health from the connectivity WebSocket (green/yellow/red).
    /// The worker mirrors its Online/Offline into `sync_status` so existing gating
    /// keeps working; the extra `Degraded` tier and the RTT live only here.
    pub connection: ConnectionHealth,
    /// Last error recorded by the sync worker (network/persist failure), kept for
    /// debugging/diagnostics. Cleared on the next successful sync. The user-facing
    /// surface for errors is the toast funnel, not this field.
    pub last_error: Option<String>,
    /// Timestamp of the last successful sync.
    pub last_synced_at: Option<DateTime<Utc>>,
}

impl ConnectionState {
    /// Whether the worker considers itself offline. The shared predicate behind the
    /// reactive `use_is_offline` slice (in halogen-ui-state) and the non-reactive
    /// `.peek()` reads (submit closures), so both spell "offline" the same way.
    pub fn is_offline(&self) -> bool {
        self.sync_status.is_offline()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Compile-time canary for the load-bearing derive (mirrors
    /// `appstate_is_partialeq`): the memo slices over this signal need
    /// `ConnectionState: PartialEq` to gate re-renders.
    #[test]
    fn connectionstate_is_partialeq() {
        fn assert_partial_eq<T: PartialEq>() {}
        assert_partial_eq::<ConnectionState>();
    }

    #[test]
    fn default_is_unknown_not_offline() {
        // Cold start is Unknown (connectivity undetermined) — it must NOT read as
        // offline, or offline-gated surfaces flash their offline states at boot.
        let d = ConnectionState::default();
        assert!(d.sync_status.is_unknown());
        assert!(!d.is_offline(), "cold start must not read as offline");
    }
}
