//! Own per-platform WebSocket transport and ticket/connect/ping/reconnect logic for rapid reachability and latency
//! updates. Emit ConnectionEvents to the worker; never write episode state from the transport.

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
