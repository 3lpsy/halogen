//! Shared response type for the WebSocket ticket endpoint (`POST /ws-ticket`). Lives here (not in the server)
//! so the API client and the server share a single definition — the client deserialises exactly what the server
//! emits. The ticket is a short-lived, WS-scoped JWT the client passes as the `?ticket=` query param on the
//! `/ws` upgrade (browsers can't set an `Authorization` header on a WebSocket handshake).

use serde::{Deserialize, Serialize};

use super::ResponsableData;

/// Response data for `POST /ws-ticket` — the short-lived ticket to hand to the
/// `/ws` upgrade.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WsTicketData {
    pub ticket: String,
}

impl ResponsableData for WsTicketData {}
