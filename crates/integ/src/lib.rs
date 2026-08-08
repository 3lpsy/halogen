//! Shared library for the Tier-1 HTTP integration journeys (`tests/*.rs`).
//!
//! Each journey drives a **real** axum server (via [`support`]) over loopback,
//! exercising routes through the real [`halogen_api::ApiClient`] — the exact
//! gateway the UI uses — so the same request-building, `serde_qs` query
//! serialisation, and envelope decoding the frontend relies on are tested on the
//! wire. The only faked dependency is upstream RSS: a [`wiremock`] server
//! reachable through a podcast's `feed_url`.
//!
//! A few journeys (media streaming, podcast artwork, SPA-fallback routing, and
//! the connectivity WebSocket `/ws-ticket` + `/ws` upgrade) talk a raw protocol
//! — binary bodies / range requests / cookie auth / unprefixed paths / `ws://`
//! frames that aren't typed `ApiClient` calls — helped by the small
//! [`Client`]/[`json_ok`] wrappers here.

mod test_fixture;

pub mod support;

pub use support::{AdminCreds, SpawnOptions, TestApp, spawn, spawn_with};

use halogen_api::{ApiClient, ApiError};
use halogen_wire::{
    DefaultListParams, EpisodeInclude, FilterParams, Includable, Order, OrderDirection, Pagination,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use url::Url;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

// ── ApiClient harness ────────────────────────────────────────────────────

/// Build the UI gateway client authed as `token`, pointed at the test server.
pub fn api(app: &TestApp, token: &str) -> ApiClient {
    let client = ApiClient::new(Url::parse(&app.base_url).expect("parse base url"));
    client.set_token(Some(token.to_string()));
    client
}

/// An unauthenticated gateway client (for 401 / public-route assertions).
pub fn anon_api(app: &TestApp) -> ApiClient {
    ApiClient::new(Url::parse(&app.base_url).expect("parse base url"))
}

/// Recover the HTTP status an [`ApiError`] corresponds to.
///
/// The server returns errors as a `ResponseData` envelope with `errors`
/// populated, which the client decodes as [`ApiError::Validation`] — dropping
/// the numeric status. Mirror the server's `extract_status_code` to recover it
/// from the error `code`, so tests can assert `status_of(&err) == 404`, etc.
pub fn status_of(err: &ApiError) -> u16 {
    match err {
        ApiError::Server { status, .. } => *status,
        ApiError::Validation(v) => {
            for fields in v.errors.values() {
                for f in fields {
                    match f.code.as_str() {
                        "exists" => return 404,
                        "unauthenticated" => return 401,
                        "unauthorized" => return 403,
                        "unique" | "conflict" => return 409,
                        "unimplemented" => return 501,
                        "panic" => return 500,
                        _ => {}
                    }
                }
            }
            400
        }
        other => panic!("expected a status-bearing ApiError, got {other:?}"),
    }
}

/// Episode list params with explicit pagination + optional order/includes/filter
/// — the exact `DefaultListParams<EpisodeInclude>` the UI's paged list builds.
pub fn ep_params(
    page: i32,
    size: i32,
    order: Option<(&str, OrderDirection)>,
    includes: Vec<EpisodeInclude>,
    filter: Option<FilterParams>,
) -> DefaultListParams<EpisodeInclude> {
    DefaultListParams {
        pagination: Some(Pagination { page, size }),
        order: order.map(|(by, direction)| Order {
            order_by: by.to_string(),
            direction,
        }),
        includes: (!includes.is_empty()).then_some(includes),
        filter,
    }
}

/// Shorthand: a single-page episode query with just pagination.
pub fn ep_page(page: i32, size: i32) -> DefaultListParams<EpisodeInclude> {
    ep_params(page, size, None, vec![], None)
}

/// A list query over any include type with no pagination param — so the server
/// applies its default page size (the silent-default path the client guards).
/// Built field-by-field (not `Default::default()`) so it works for include
/// types that don't implement `Default`, like `NoInclude`.
pub fn list_default<T: Includable + Serialize + DeserializeOwned>() -> DefaultListParams<T> {
    DefaultListParams {
        pagination: None,
        order: None,
        includes: None,
        filter: None,
    }
}

/// A list query asking for one big page (size 200) — enough to pull a whole
/// small test library in one request.
pub fn list_all<T: Includable + Serialize + DeserializeOwned>() -> DefaultListParams<T> {
    DefaultListParams {
        pagination: Some(Pagination { page: 0, size: 200 }),
        order: None,
        includes: None,
        filter: None,
    }
}

// ── RSS fixtures + arrange helpers ─────────────────────────────────────────

/// Load an RSS fixture from the shared `data/tests` corpus.
pub fn load_feed(name: &str) -> String {
    let path = format!("{}/../../data/tests/{}", env!("CARGO_MANIFEST_DIR"), name);
    std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("feed fixture not found: {path}"))
}

/// A wiremock server that serves `feed` for any GET — the only upstream we fake.
pub async fn mock_feed(feed: &str) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string(feed.to_string()))
        .mount(&server)
        .await;
    server
}

/// Arrange-helper used by several journeys: subscribe to a mocked feed, poll
/// once via the gateway, and return `(podcast_id, first_episode_id)`. `client`
/// must be an admin-authed [`ApiClient`] (poll is admin-only).
pub async fn subscribe_and_poll(app: &TestApp, client: &ApiClient, feed_name: &str) -> (i32, i32) {
    let feed = load_feed(feed_name);
    let upstream = mock_feed(&feed).await;
    let podcast_id = app.seed_podcast("Test Podcast", &upstream.uri()).await;

    client.poll_now().await.expect("poll");

    // Pull this podcast's episodes through the same list endpoint the UI uses,
    // newest-first, so the "first" episode is deterministic.
    let page = client
        .list_episodes(ep_params(
            0,
            50,
            Some(("published_at", OrderDirection::Desc)),
            vec![],
            Some(FilterParams {
                podcast_id: Some(podcast_id),
                ..Default::default()
            }),
        ))
        .await
        .expect("list episodes");
    let episode_id = page.data.first().expect("at least one ingested episode").id;
    // `upstream` is no longer needed — the poll already fetched the feed.
    (podcast_id, episode_id)
}

// ── Raw-HTTP helpers (media-streaming + SPA-fallback journeys only) ─────────

/// Thin reqwest wrapper carrying the base URL + an optional bearer token. Used
/// by the two journeys that aren't typed `ApiClient` calls.
pub struct Client {
    http: reqwest::Client,
    base: String,
    token: Option<String>,
}

impl Client {
    pub fn new(app: &TestApp) -> Self {
        Self {
            http: reqwest::Client::new(),
            base: format!("{}/api/v1", app.base_url),
            token: None,
        }
    }

    pub fn with_token(mut self, token: &str) -> Self {
        self.token = Some(token.to_string());
        self
    }

    fn auth(&self, rb: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.token {
            Some(t) => rb.bearer_auth(t),
            None => rb,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base, path)
    }

    pub async fn get(&self, path: &str) -> reqwest::Response {
        self.auth(self.http.get(self.url(path)))
            .send()
            .await
            .expect("GET request")
    }
}

/// Parse a response body as JSON, asserting a 2xx status first.
pub async fn json_ok(resp: reqwest::Response) -> Value {
    let status = resp.status();
    let body = resp.text().await.expect("body text");
    assert!(status.is_success(), "expected 2xx, got {status}: {body}");
    serde_json::from_str(&body).expect("json body")
}
