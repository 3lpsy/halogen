//! Browser-free `/healthz` probe of the server's embedded-frontend fallback, isolating server reachability from WASM
//! connection bugs. Runs without Chrome or ignore, but requires embed-frontend and dist/ produced by `just ui-build`.

use halogen_integ::support;

/// Root `/healthz` must 200 with `{ data: { running: true } }` even though the
/// embedded SPA fallback is mounted — the explicit route has to win over the
/// fallback, or the wasm login page's health probe falls through to index.html
/// and fails to deserialize.
#[tokio::test]
async fn healthz_reachable_on_embed_server() {
    let app = support::spawn().await;

    let resp = reqwest::Client::new()
        .get(format!("{}/healthz", app.base_url))
        .send()
        .await
        .expect("GET /healthz");

    let status = resp.status();
    let ctype = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let body = resp.text().await.unwrap_or_default();

    assert_eq!(
        status, 200,
        "healthz status; content-type={ctype}; body:\n{body}"
    );
    let json: serde_json::Value = serde_json::from_str(&body)
        .unwrap_or_else(|e| panic!("healthz not JSON ({e}); content-type={ctype}; body:\n{body}"));
    assert_eq!(
        json["data"]["running"].as_bool(),
        Some(true),
        "healthz payload; body:\n{body}"
    );
}
