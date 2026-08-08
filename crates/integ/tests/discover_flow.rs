//! Discover journey — federated podcast search through the real server.
//!
//! The Discover providers (iTunes, gpodder) are backed here by `wiremock`
//! upstreams serving the saved `data/tests/discover_*_search.json` fixtures, so
//! the real `/api/v1/discover/*` path — auth, the `DiscoverService`, provider
//! parsing, the merge/fault-tolerance logic, and `ApiClient` decoding — runs end
//! to end without any external call.
//!
//! Run with: `cargo nextest run -p halogen-integ -E 'binary(discover_flow)'`

use halogen_integ::*;
use halogen_wire::{DiscoverProvider, DiscoverSearchParams};
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Spawn a server whose Discover providers are backed by wiremock upstreams that
/// serve the saved provider fixtures. Returns the app plus the two mock servers
/// (kept alive for the test's lifetime).
async fn spawn_with_discover() -> (TestApp, MockServer, MockServer) {
    let itunes = mock_feed(&load_feed("discover_itunes_search.json")).await;
    let gpodder = mock_feed(&load_feed("discover_gpodder_search.json")).await;
    let app = spawn_with(SpawnOptions {
        discover_itunes_base_url: Some(itunes.uri()),
        discover_gpodder_base_url: Some(gpodder.uri()),
        ..Default::default()
    })
    .await;
    (app, itunes, gpodder)
}

fn search(q: &str, providers: Option<Vec<DiscoverProvider>>) -> DiscoverSearchParams {
    DiscoverSearchParams {
        q: q.to_string(),
        providers,
    }
}

#[tokio::test]
async fn discover_lists_providers_and_merges_results() {
    let (app, _itunes, _gpodder) = spawn_with_discover().await;
    let admin = app.seed_admin().await;
    let client = api(&app, &admin.token);

    // The providers endpoint advertises both, available.
    let providers = client.discover_providers().await.expect("providers");
    assert_eq!(providers.providers.len(), 2);
    assert!(providers.providers.iter().all(|p| p.available));

    // Search merges both providers (3 + 3), each row tagged with its source and
    // carrying a title + feed url + a provider-scoped synthetic id. No artwork
    // field exists on the type at all.
    let data = client
        .discover_search(search("rust", None))
        .await
        .expect("search");
    assert!(
        data.errors.is_empty(),
        "no provider failed: {:?}",
        data.errors
    );
    assert_eq!(data.items.len(), 6, "3 iTunes + 3 gpodder, merged");
    assert!(
        data.items
            .iter()
            .any(|i| i.provider == DiscoverProvider::Itunes)
    );
    assert!(
        data.items
            .iter()
            .any(|i| i.provider == DiscoverProvider::Gpodder)
    );
    for item in &data.items {
        assert!(!item.title.is_empty(), "row has a title");
        assert!(item.feed_url.starts_with("http"), "row has a feed url");
        assert!(
            item.id.starts_with(item.provider.as_str()),
            "id is provider-scoped"
        );
    }
}

#[tokio::test]
async fn discover_filters_to_selected_provider() {
    let (app, _itunes, _gpodder) = spawn_with_discover().await;
    let admin = app.seed_admin().await;
    let client = api(&app, &admin.token);

    let data = client
        .discover_search(search("rust", Some(vec![DiscoverProvider::Gpodder])))
        .await
        .expect("search");
    assert_eq!(data.items.len(), 3);
    assert!(
        data.items
            .iter()
            .all(|i| i.provider == DiscoverProvider::Gpodder)
    );
}

#[tokio::test]
async fn discover_is_fault_tolerant_when_a_provider_fails() {
    // iTunes serves its fixture; the gpodder upstream returns 500.
    let itunes = mock_feed(&load_feed("discover_itunes_search.json")).await;
    let gpodder = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&gpodder)
        .await;
    let app = spawn_with(SpawnOptions {
        discover_itunes_base_url: Some(itunes.uri()),
        discover_gpodder_base_url: Some(gpodder.uri()),
        ..Default::default()
    })
    .await;
    let admin = app.seed_admin().await;
    let client = api(&app, &admin.token);

    // The search still succeeds (HTTP 200): iTunes results come back, and the
    // gpodder failure surfaces as a per-provider error rather than blanking it.
    let data = client
        .discover_search(search("rust", None))
        .await
        .expect("search ok");
    assert_eq!(data.items.len(), 3, "iTunes results survive");
    assert!(
        data.items
            .iter()
            .all(|i| i.provider == DiscoverProvider::Itunes)
    );
    assert_eq!(data.errors.len(), 1);
    assert_eq!(data.errors[0].provider, DiscoverProvider::Gpodder);
}

#[tokio::test]
async fn discover_search_rejects_empty_query() {
    let (app, _i, _g) = spawn_with_discover().await;
    let admin = app.seed_admin().await;
    let client = api(&app, &admin.token);

    let err = client
        .discover_search(search("   ", None))
        .await
        .expect_err("blank query is a request error");
    assert_eq!(status_of(&err), 400);
}

#[tokio::test]
async fn discover_requires_auth() {
    let (app, _i, _g) = spawn_with_discover().await;
    let client = anon_api(&app);

    let err = client
        .discover_search(search("rust", None))
        .await
        .expect_err("unauthenticated search is rejected");
    assert_eq!(status_of(&err), 401);
}
