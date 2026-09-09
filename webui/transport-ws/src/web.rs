//! Web ([`gloo_net`]) [`WsConn`] — the browser `WebSocket` exposed as a futures
//! Stream/Sink. wasm-only; no TLS in the dependency graph (the browser does the
//! `wss://` handshake itself).

use async_trait::async_trait;
use futures::{SinkExt, StreamExt};
use gloo_net::websocket::Message;
use gloo_net::websocket::futures::WebSocket;

use super::WsConn;
use halogen_webui_logging::debug;

/// One open browser WebSocket. Dropping it closes the socket (gloo handles the
/// close handshake), which is how the driver's cancellation tears the link down.
struct WebWsConn {
    ws: WebSocket,
}

#[async_trait(?Send)]
impl WsConn for WebWsConn {
    async fn send(&mut self, text: &str) -> bool {
        self.ws.send(Message::Text(text.to_string())).await.is_ok()
    }

    async fn recv(&mut self) -> Option<String> {
        // Skip non-text frames; surface a close/error as the stream ending.
        loop {
            match self.ws.next().await {
                Some(Ok(Message::Text(text))) => return Some(text),
                Some(Ok(Message::Bytes(_))) => continue,
                Some(Err(_)) | None => return None,
            }
        }
    }
}

/// Open a connection. `Some` once the handshake has *started* — liveness is then
/// confirmed by the driver's first pong (the browser API gives no awaitable open).
pub(super) async fn connect(url: &str) -> Option<Box<dyn WsConn>> {
    match WebSocket::open(url) {
        Ok(ws) => Some(Box::new(WebWsConn { ws })),
        Err(e) => {
            debug!(error = %e, "WS open failed");
            None
        }
    }
}
