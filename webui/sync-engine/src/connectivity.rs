//! Connectivity: the WebSocket driver lifecycle + connection/latency status. A focused `impl SyncService` block,
//! everything here is about *reachability* (is the server up, how laggy), distinct from the data-sync command handling
//! in `worker.rs`.

use std::rc::Rc;

use super::{Command, DEGRADED_RTT_MS, RTT_WINDOW, SyncService, spawn_task};
use halogen_webui_app_state::state::{ConnectionHealth, SyncStatus};
use halogen_webui_component_toast::ToastDecision;
use halogen_webui_logging::debug;
use halogen_webui_transport_ws::{ConnectionEvent, EventSink};

impl SyncService {
    /// Spawn the connectivity-WebSocket driver (tearing down any existing one first). No-op when manually offline or
    /// before auth, there's nothing to connect to. Re-invoked on every `SetAuth` so the driver always mints tickets
    /// with the current token. The driver reports back via the internal channel as `Ws*` commands.
    pub(super) fn start_ws(&mut self) {
        self.stop_ws();
        if self.manual_offline {
            return;
        }
        let (Some(api), Some(tx)) = (
            self.api_client.as_ref().map(|a| a.clone_handle()),
            self.internal_tx.clone(),
        ) else {
            return;
        };
        #[cfg(not(target_arch = "wasm32"))]
        if api.is_local() {
            // The initial and periodic pulls probe the local session; no socket is needed.
            return;
        }
        let emit: EventSink = Rc::new(move |event| {
            let cmd = match event {
                ConnectionEvent::Connected => Command::WsConnected,
                ConnectionEvent::Disconnected => Command::WsDisconnected,
                ConnectionEvent::Pong { rtt_ms } => Command::WsLatency { rtt_ms },
                ConnectionEvent::AuthExpired => Command::WsAuthExpired,
            };
            let _ = tx.unbounded_send(cmd);
        });
        let (cancel, driver) = halogen_webui_transport_ws::spawn_handle(api, emit);
        if spawn_task(driver) {
            self.ws_cancel = Some(cancel);
        } else {
            debug!(
                "WS driver unsupported on this runtime (no dioxus runtime or LocalSet); connectivity via pull only"
            );
        }
    }

    /// Stop the connectivity-WebSocket driver, if any (closes its socket).
    pub(super) fn stop_ws(&mut self) {
        if let Some(cancel) = self.ws_cancel.take() {
            let _ = cancel.send(());
        }
    }

    /// Mark the server reachable. Lifts connectivity out of `Offline`/`Unknown`
    /// (preserving a known `Degraded`/RTT reading from the WS) and keeps
    /// `sync_status` in lockstep for the download/sync gating that still reads it.
    pub(super) fn set_online_status(&mut self) {
        self.connection.sync_status = SyncStatus::Online;
        if !self.connection.connection.is_online() {
            self.connection.connection = ConnectionHealth::Online { rtt_ms: 0 };
        }
    }

    /// Mark the server unreachable across both the connection model and the legacy
    /// `sync_status`, and forget stale latency samples.
    pub(super) fn set_offline_status(&mut self) {
        self.connection.connection = ConnectionHealth::Offline;
        self.connection.sync_status = SyncStatus::Offline;
        self.rtt_samples.clear();
    }

    /// Fold one pong RTT into the smoothed window and recompute Online vs Degraded.
    /// A pong only arrives over a live socket, so this also asserts reachability.
    pub(super) fn record_latency(&mut self, rtt_ms: u32) {
        // A pong proves reachability: lift a stale `Offline` / cold-start `Unknown`
        // sync_status (is_offline gating). Never stomp an in-progress `Syncing`.
        if matches!(
            self.connection.sync_status,
            SyncStatus::Offline | SyncStatus::Unknown
        ) {
            self.connection.sync_status = SyncStatus::Online;
        }
        self.rtt_samples.push(rtt_ms);
        if self.rtt_samples.len() > RTT_WINDOW {
            self.rtt_samples.remove(0);
        }
        let avg = self.rtt_samples.iter().sum::<u32>() / self.rtt_samples.len() as u32;
        self.connection.connection = if avg > DEGRADED_RTT_MS {
            ConnectionHealth::Degraded { rtt_ms: avg }
        } else {
            ConnectionHealth::Online { rtt_ms: avg }
        };
    }

    /// Apply a classified error decision: raise the right toast and, for a dead
    /// token, flag `auth_expired` so the provider signs out. `Offline`/`Silent`
    /// are no-ops here (the caller reflects Offline in `sync_status`).
    pub(super) fn apply_decision(&mut self, decision: ToastDecision) {
        match decision {
            ToastDecision::SignOut => {
                self.toasts.error("Session expired — please sign in again.");
                self.session.auth_expired = true;
            }
            other => {
                self.toasts.apply(other);
            }
        }
    }
}
