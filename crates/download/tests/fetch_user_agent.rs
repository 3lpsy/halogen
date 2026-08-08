//! The resolved `server_fetch_user_agent` must reach the WIRE, not just `Config`.
//! Loopback only — no network. Each nextest test is its own process, so the
//! process-global override and the memoized client can't leak between tests.

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

/// Serve one request, returning the `User-Agent` header it carried.
async fn capture_user_agent(listener: TcpListener) -> Option<String> {
    let (stream, _) = listener.accept().await.expect("accept");
    let mut reader = BufReader::new(stream);
    let mut ua = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).await.expect("read") == 0 || line == "\r\n" {
            break;
        }
        if let Some(v) = line
            .strip_prefix("user-agent: ")
            .or(line.strip_prefix("User-Agent: "))
        {
            ua = Some(v.trim().to_string());
        }
    }
    reader
        .into_inner()
        .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
        .await
        .expect("respond");
    ua
}

#[tokio::test]
async fn configured_user_agent_reaches_the_wire() {
    halogen_net::configure_user_agent(Some("MyPodcatcher/2.0".to_string()));

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let url = format!("http://{}/ep.mp3", listener.local_addr().expect("addr"));
    let server = tokio::spawn(capture_user_agent(listener));

    // Built AFTER the override — the client memoizes its UA on first use.
    let _ = halogen_download::download_client().get(&url).send().await;

    assert_eq!(
        server.await.expect("server task").as_deref(),
        Some("MyPodcatcher/2.0"),
        "the deployer's server_fetch_user_agent must be what origins see"
    );
}

#[tokio::test]
async fn default_user_agent_reaches_the_wire() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let url = format!("http://{}/ep.mp3", listener.local_addr().expect("addr"));
    let server = tokio::spawn(capture_user_agent(listener));

    let _ = halogen_download::download_client().get(&url).send().await;

    // Unconfigured: an honest, purpose-tagged identity — never absent, which is
    // what media CDNs behind bot management reject.
    let ua = server
        .await
        .expect("server task")
        .expect("a User-Agent was sent");
    assert!(
        ua.starts_with("Halogen/") && ua.ends_with(" (+podcast-download)"),
        "unexpected default on the wire: {ua}"
    );
}
