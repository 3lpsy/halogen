//! Connectivity WebSocket transport + reconnect driver.
//!
//! The worker keeps one socket open so it learns within ~1s (not up to the 60s
//! pull interval) when the server becomes unreachable and when it comes back, and
//! measures round-trip latency for the "degraded" indicator. This module owns the
//! *transport* (a tiny per-platform [`WsConn`]) and the *driver* (mint-ticket →
//! connect → app-level ping/pong → reconnect-with-backoff) — both platform-
//! agnostic above the [`connect`] boundary, mirroring the `MediaStore`/`LocalStore`
//! split.
//!
//! The driver emits [`ConnectionEvent`]s back to the sync worker (mapped to
//! `Command`s) and never writes `EpisodeState` itself — the worker stays the single
//! writer.

#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(target_arch = "wasm32")]
mod web;

mod backoff;
mod conn;
mod driver;

#[cfg(not(target_arch = "wasm32"))]
use native::connect;
#[cfg(target_arch = "wasm32")]
use web::connect;

pub use conn::{ConnectionEvent, EventSink, WsConn};
pub use driver::spawn_handle;
