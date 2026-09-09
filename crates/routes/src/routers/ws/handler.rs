//! Mint short-lived WS tickets through bearer-authenticated POST /ws-ticket; GET /ws validates its query ticket outside
//! bearer middleware because browser handshakes cannot set that header. Echo {t:ping,ts:N} as pong for reachability and
//! RTT; no unsolicited server messages yet.

use std::time::Duration;

use axum::{
    Extension, Json,
    extract::{
        Query,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::StatusCode,
    response::{IntoResponse, Response},
};
use halogen_wire::{ResponseData, WsTicketData};
use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::routers::auth::token::{decode_claims, mint_ws_ticket};
use crate::routers::errors::ApiError;
use crate::routers::extractors::AuthUserId;
use crate::routers::middleware::AuthConfig;

/// How long a minted WS ticket stays valid. Short on purpose — it rides in the
/// upgrade URL's query string, and the client re-mints on every (re)connect, so a
/// tight window keeps the replay surface minimal.
const WS_TICKET_TTL_SECS: u64 = 30;

/// Drop a socket whose peer has gone silent. The client pings ~every 15s; allow a
/// few missed beats (covers a slow/janky link) before assuming the peer is dead.
const IDLE_TIMEOUT: Duration = Duration::from_secs(45);

/// `POST /ws-ticket` — mint a short-lived WS ticket for the authenticated user.
/// Behind the bearer middleware, so `AuthUserId` is the caller's own id.
#[axum::debug_handler]
pub async fn ticket(
    AuthUserId(user_id): AuthUserId,
    Extension(auth): Extension<AuthConfig>,
) -> Result<Json<ResponseData<WsTicketData>>, ApiError> {
    let ticket = mint_ws_ticket(&auth.secret, &user_id.to_string(), WS_TICKET_TTL_SECS)?;
    Ok(Json(ResponseData::from_data(WsTicketData { ticket })))
}

/// Query string on the `/ws` upgrade. `ticket` is optional at the type level so a
/// missing one yields our own `401` rather than a generic extractor rejection.
#[derive(Debug, Deserialize)]
pub struct TicketParams {
    pub ticket: Option<String>,
}

/// The authenticated user id when `ticket` is a valid, unexpired, WS-scoped token,
/// else `None`. Scope-locked: only `is_ws()` tokens are accepted here, and the API
/// bearer middleware rejects `is_ws()` tokens — so neither credential works on the
/// other path (mirrors `media_auth`).
pub fn ws_ticket_user(ticket: Option<&str>, auth: &AuthConfig) -> Option<i32> {
    let claims = decode_claims(ticket?, &auth.secret)
        .ok()
        .filter(|c| c.is_ws())?;
    claims.sub.parse::<i32>().ok()
}

/// `GET /ws` — validate the ticket, then upgrade. Mounted outside the bearer
/// middleware; auth comes from `?ticket=` only.
pub async fn ws(
    Query(params): Query<TicketParams>,
    Extension(auth): Extension<AuthConfig>,
    upgrade: WebSocketUpgrade,
) -> Response {
    let Some(user_id) = ws_ticket_user(params.ticket.as_deref(), &auth) else {
        return (StatusCode::UNAUTHORIZED, "Invalid or expired ticket").into_response();
    };
    debug!(user_id, "WebSocket connection authorized");
    upgrade.on_upgrade(handle_socket)
}

/// Inbound frame from the client. Only `ping` is acted on today.
#[derive(Debug, Deserialize)]
struct ClientMsg {
    t: String,
    #[serde(default)]
    ts: Option<i64>,
}

/// Outbound frame to the client.
#[derive(Debug, Serialize)]
struct ServerMsg {
    t: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    ts: Option<i64>,
}

/// Build the reply for an inbound text frame: echo a `pong` (carrying the client's
/// `ts` so it can compute RTT) for a `ping`, otherwise nothing.
fn reply_for(text: &str) -> Option<String> {
    let msg: ClientMsg = serde_json::from_str(text).ok()?;
    (msg.t == "ping")
        .then(|| {
            serde_json::to_string(&ServerMsg {
                t: "pong",
                ts: msg.ts,
            })
            .ok()
        })
        .flatten()
}

/// Per-connection loop: answer pings, drop the socket if the peer goes silent past
/// [`IDLE_TIMEOUT`] or closes/errors.
async fn handle_socket(mut socket: WebSocket) {
    loop {
        match tokio::time::timeout(IDLE_TIMEOUT, socket.recv()).await {
            // Peer closed, errored, or the stream ended.
            Ok(None) | Ok(Some(Err(_))) => break,
            // No traffic within the window — assume the peer is gone.
            Err(_) => {
                let _ = socket.send(Message::Close(None)).await;
                break;
            }
            Ok(Some(Ok(message))) => match message {
                Message::Text(text) => {
                    if let Some(reply) = reply_for(text.as_str())
                        && socket.send(Message::Text(reply.into())).await.is_err()
                    {
                        break;
                    }
                }
                Message::Close(_) => break,
                // Binary / protocol Ping / Pong: ignore (tungstenite auto-answers
                // protocol pings; our keepalive is the app-level ping above).
                _ => {}
            },
        }
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
