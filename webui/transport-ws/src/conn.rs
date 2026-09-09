//! Connectivity transport contract: the per-platform [`WsConn`] socket plus the
//! [`ConnectionEvent`]s the driver reports back to the worker.

use std::rc::Rc;

use async_trait::async_trait;

/// Events the driver reports to the worker. The worker maps each to a `Command`.
#[derive(Debug, Clone, Copy)]
pub enum ConnectionEvent {
    /// Socket opened (after a successful ticket mint + upgrade).
    Connected,
    /// Socket closed / errored / went silent, or a (re)connect attempt failed.
    Disconnected,
    /// A pong came back; `rtt_ms` is this single round-trip.
    Pong { rtt_ms: u32 },
    /// Ticket mint returned 401 — the session is dead, stop and let the worker
    /// sign out. (A transport failure on mint is just `Disconnected` + retry.)
    AuthExpired,
}

/// Sink the driver pushes [`ConnectionEvent`]s into (the worker supplies a closure
/// that forwards them as `Command`s on its internal channel).
pub type EventSink = Rc<dyn Fn(ConnectionEvent)>;

/// One open connection's send/receive half. Per-platform (`web`/`native`); the
/// driver above is written once against this.
#[async_trait(?Send)]
pub trait WsConn {
    /// Send a text frame. `false` = the socket is broken (driver reconnects).
    async fn send(&mut self, text: &str) -> bool;

    /// Await the next text frame. `None` = the socket closed or errored. Non-text
    /// frames (binary / protocol ping-pong) are swallowed internally.
    async fn recv(&mut self) -> Option<String>;
}
