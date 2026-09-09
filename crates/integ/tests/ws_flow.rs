//! Mint a WebSocket ticket, upgrade over loopback, and exchange ping/pong. Missing tickets and normal API tokens must
//! return 401 without upgrading. Run the halogen-integ ws_flow binary.

use futures_util::{SinkExt, StreamExt};
use halogen_integ::*;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Error as WsError;
use tokio_tungstenite::tungstenite::Message;

/// Connect, read frames until the next text one (skipping any control/binary).
async fn next_text<S>(sock: &mut S) -> String
where
    S: StreamExt<Item = Result<Message, WsError>> + Unpin,
{
    loop {
        match sock.next().await.expect("a frame").expect("frame ok") {
            Message::Text(t) => return t.as_str().to_string(),
            _ => continue,
        }
    }
}

/// Assert a handshake attempt is rejected with HTTP 401 (no upgrade).
async fn assert_handshake_401(url: &str) {
    match connect_async(url).await {
        Err(WsError::Http(resp)) => {
            assert_eq!(resp.status().as_u16(), 401, "handshake should be 401");
        }
        Err(other) => panic!("expected an HTTP 401 rejection, got {other:?}"),
        Ok(_) => panic!("handshake unexpectedly succeeded for {url}"),
    }
}

#[tokio::test]
async fn ws_ticket_and_socket_journey() {
    let app = spawn().await;
    let admin = app.seed_admin().await;
    let client = api(&app, &admin.token);

    // 1) Mint a ticket (authed POST /ws-ticket).
    let ticket = client.mint_ws_ticket().await.expect("mint ticket").ticket;
    assert!(!ticket.is_empty(), "ticket should be a non-empty JWT");

    // 2) Upgrade with the valid ticket → 101, then app-level ping echoes a pong
    //  carrying our `ts` (the client uses that to compute RTT).
    let good_url = format!("{}?ticket={}", client.ws_url(), ticket);
    let (mut sock, resp) = connect_async(good_url.as_str()).await.expect("ws upgrade");
    assert_eq!(resp.status().as_u16(), 101, "upgrade switches protocols");

    sock.send(Message::Text(r#"{"t":"ping","ts":4242}"#.into()))
        .await
        .expect("send ping");
    assert_eq!(next_text(&mut sock).await, r#"{"t":"pong","ts":4242}"#);
    sock.close(None).await.ok();

    // 3) No ticket → rejected at the handshake (401).
    assert_handshake_401(&format!("{}", client.ws_url())).await;

    // 4) A normal API token is NOT a WS ticket (scope-locked) → 401.
    let api_as_ticket = format!("{}?ticket={}", client.ws_url(), admin.token);
    assert_handshake_401(&api_as_ticket).await;

    // 5) A garbage ticket → 401.
    let garbage = format!("{}?ticket=not-a-jwt", client.ws_url());
    assert_handshake_401(&garbage).await;
}
