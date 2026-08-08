//! Reconnect driver: mint-ticket → connect → app-level ping/pong → reconnect with
//! [`Backoff`]. Platform-agnostic above the [`connect`](super::connect) boundary.
//!
//! Why app-level ping/pong (not protocol frames): browsers don't expose WebSocket
//! protocol ping/pong to JS, so the only portable way to measure RTT — and to
//! notice a *silent* drop (wifi gone, no TCP FIN) — is an application message.
//! The client sends `{"t":"ping","ts":N}` on a timer; the server echoes
//! `{"t":"pong","ts":N}`; RTT is `now - ts`.

use futures::channel::oneshot;
use futures::future::{Either, select};
use halogen_api::{ApiClient, ApiError};

use super::backoff::Backoff;
use super::connect;
use super::{ConnectionEvent, EventSink, WsConn};
use halogen_ui_logging::{debug, warn};
use halogen_ui_platform::time::sleep_ms;

/// How often the client sends an app-level ping (and so how often RTT refreshes).
const PING_INTERVAL_MS: u32 = 15_000;
/// Consecutive unanswered pings before we treat the socket as dead and reconnect.
/// Covers a *silent* drop the OS never surfaced as a close; a clean close is seen
/// immediately via the receive stream ending.
const MAX_MISSED_PONGS: u32 = 2;

/// Minimum time a connection must have stayed up before a disconnect resets the
/// backoff. A misbehaving server that accepts, pongs once, then immediately drops
/// would otherwise reset every cycle and hammer reconnects (~1s loops) forever; we
/// only treat a session as "healthy enough to reset" once it outlived this.
const MIN_LIVE_SESSION_MS: i64 = 10_000;

/// Build the cancel channel + driver future for the worker to run via its
/// `spawn_task`. Returns the cancel sender — drop it (or send `()`) to stop the
/// driver, which drops any open socket.
pub fn spawn_handle(
    api: ApiClient,
    emit: EventSink,
) -> (oneshot::Sender<()>, impl std::future::Future<Output = ()>) {
    let (tx, rx) = oneshot::channel();
    (tx, run_driver(api, emit, rx))
}

/// Run the connectivity driver until `cancel` fires (the worker drops its sender on
/// sign-out / manual-offline / re-auth). Reconnects forever with backoff; the only
/// terminal exit is a 401 on ticket mint (→ [`ConnectionEvent::AuthExpired`]).
///
/// `api` is an independent client handle (cloned base + token snapshot), so a
/// later token refresh means the worker cancels this driver and spawns a fresh one
/// — the re-mint on every connect therefore always uses a current token.
pub async fn run_driver(api: ApiClient, emit: EventSink, cancel: oneshot::Receiver<()>) {
    let driver = drive(api, emit);
    futures::pin_mut!(driver);
    // Whichever resolves first wins; if `cancel` wins, `driver` is dropped here,
    // which drops any open `WsConn` and closes the socket.
    let _ = select(driver, cancel).await;
    debug!("WS driver stopped");
}

/// The reconnect loop (raced against cancellation by [`run_driver`]).
async fn drive(api: ApiClient, emit: EventSink) {
    let mut backoff = Backoff::new();
    loop {
        // 1. Mint a fresh ticket (also our auth + reachability probe).
        let ticket = match api.mint_ws_ticket().await {
            Ok(data) => data.ticket,
            Err(ApiError::Server { status: 401, .. }) => {
                warn!("WS ticket mint unauthorized — signing out");
                emit(ConnectionEvent::AuthExpired);
                return;
            }
            Err(e) => {
                debug!(error = %e, "WS ticket mint failed — offline, will retry");
                emit(ConnectionEvent::Disconnected);
                backoff.wait().await;
                continue;
            }
        };

        // 2. Open the socket (`ws_url` keeps the ticket out of any logged value).
        // On the web `connect` can't await the handshake, so a "success" here isn't
        // yet proof of liveness — `session` confirms that via the first pong.
        let url = format!("{}?ticket={}", api.ws_url(), ticket);
        let session_start = now_ms();
        let confirmed = match connect(&url).await {
            Some(conn) => session(conn, &emit).await,
            None => false,
        };

        // Whatever happened, we're now disconnected. Reset backoff ONLY when the
        // connection was confirmed live (a first pong) AND stayed up for at least
        // `MIN_LIVE_SESSION_MS` — otherwise a web socket that "opens" against a down
        // server, or a confirm-then-drop server that pongs once and bails, would
        // reset every cycle and hammer reconnects instead of backing off.
        emit(ConnectionEvent::Disconnected);
        if confirmed && now_ms() - session_start >= MIN_LIVE_SESSION_MS {
            backoff.reset();
        }
        backoff.wait().await;
    }
}

/// Drive one live connection: ping on a timer, surface pong RTTs, and return when
/// the socket closes/errors or the peer goes silent past [`MAX_MISSED_PONGS`].
///
/// Returns `true` once at least one pong proved the link live (we emit
/// [`ConnectionEvent::Connected`] on that first pong, not on mere socket-open, so
/// "online" always means a real round-trip succeeded).
async fn session(mut conn: Box<dyn WsConn>, emit: &EventSink) -> bool {
    let mut connected = false;

    // Probe immediately so liveness (and the first RTT) is known within one round
    // trip rather than after a full ping interval.
    if !conn.send(&ping_frame()).await {
        return connected;
    }
    // Count of pings sent without a matching pong. A pong decrements it by one
    // (not a reset to zero), so a *stale* pong for an old ping can't mask ongoing
    // silence — the deficit keeps climbing until `MAX_MISSED_PONGS` reconnects.
    let mut outstanding: u32 = 1;

    loop {
        // Race the next inbound frame against the ping timer. Scoped so the `&mut
        // conn` borrow from `recv` ends before we call `conn.send` below.
        let got_frame = {
            let recv = conn.recv();
            let tick = sleep_ms(PING_INTERVAL_MS);
            futures::pin_mut!(recv, tick);
            match select(recv, tick).await {
                Either::Left((frame, _)) => Some(frame),
                Either::Right(((), _)) => None,
            }
        };

        match got_frame {
            // Inbound frame.
            Some(Some(text)) => {
                if let Some(rtt_ms) = pong_rtt(&text) {
                    outstanding = outstanding.saturating_sub(1);
                    if !connected {
                        connected = true;
                        emit(ConnectionEvent::Connected);
                    }
                    emit(ConnectionEvent::Pong { rtt_ms });
                }
            }
            // Stream ended — clean close or error.
            Some(None) => break,
            // Ping timer fired.
            None => {
                if outstanding >= MAX_MISSED_PONGS {
                    debug!(outstanding, "WS peer silent — reconnecting");
                    break;
                }
                if !conn.send(&ping_frame()).await {
                    break;
                }
                outstanding += 1;
            }
        }
    }
    connected
}

/// Current wall-clock in ms, for stamping pings (echoed by the server so we can
/// compute RTT). Cross-target; magnitude only needs to be self-consistent.
fn now_ms() -> i64 {
    #[cfg(target_arch = "wasm32")]
    {
        js_sys::Date::now() as i64
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    }
}

/// Parse a server frame; return the RTT if it's a `pong` carrying our `ts`.
fn pong_rtt(text: &str) -> Option<u32> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    if v.get("t")?.as_str()? != "pong" {
        return None;
    }
    let ts = v.get("ts")?.as_i64()?;
    Some((now_ms() - ts).max(0) as u32)
}

/// A `{"t":"ping","ts":<now>}` frame stamped with the current time (the server
/// echoes `ts` so we compute RTT on the matching pong).
fn ping_frame() -> String {
    format!(r#"{{"t":"ping","ts":{}}}"#, now_ms())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pong_rtt_parses_and_is_non_negative() {
        // A pong echoing a ts slightly in the past yields a small, non-negative RTT.
        let ts = now_ms() - 5;
        let frame = format!(r#"{{"t":"pong","ts":{ts}}}"#);
        let rtt = pong_rtt(&frame).expect("rtt");
        assert!(rtt < 60_000, "rtt should be a sane small number, got {rtt}");
    }

    #[test]
    fn pong_rtt_ignores_non_pong_and_garbage() {
        assert!(pong_rtt(r#"{"t":"ping","ts":1}"#).is_none());
        assert!(pong_rtt(r#"{"t":"pong"}"#).is_none()); // no ts
        assert!(pong_rtt("not json").is_none());
    }
}
