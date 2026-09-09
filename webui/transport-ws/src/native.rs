//! Native ([`tokio_tungstenite`]) [`WsConn`] — desktop/mobile. TLS is rustls only
//! (`rustls-tls-webpki-roots`); `connect_async` awaits the full handshake, so a
//! returned connection is genuinely open (unlike the web path).

use async_trait::async_trait;
use futures::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};

use super::WsConn;
use halogen_webui_logging::debug;

/// One open native WebSocket. Dropping it drops the underlying stream, closing the
/// socket — how the driver's cancellation tears the link down.
struct NativeWsConn {
    stream: WebSocketStream<MaybeTlsStream<TcpStream>>,
}

#[async_trait(?Send)]
impl WsConn for NativeWsConn {
    async fn send(&mut self, text: &str) -> bool {
        self.stream
            .send(Message::Text(text.to_string().into()))
            .await
            .is_ok()
    }

    async fn recv(&mut self) -> Option<String> {
        // Skip control/binary frames; surface a close/error as the stream ending.
        loop {
            match self.stream.next().await {
                Some(Ok(Message::Text(text))) => return Some(text.as_str().to_string()),
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => return None,
                Some(Ok(_)) => continue,
            }
        }
    }
}

/// Open a connection, awaiting the handshake. `None` on any connect/TLS failure.
pub(super) async fn connect(url: &str) -> Option<Box<dyn WsConn>> {
    match connect_async(url).await {
        Ok((stream, _resp)) => Some(Box::new(NativeWsConn { stream })),
        Err(e) => {
            debug!(error = %e, "WS connect failed");
            None
        }
    }
}
