use super::*;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Load a provider JSON fixture from `data/tests/<name>` (repo root is two
/// levels up from this crate's manifest dir, same as every other crate).
fn load_fixture(name: &str) -> String {
    let path = format!("{}/../../data/tests/{}", env!("CARGO_MANIFEST_DIR"), name);
    std::fs::read_to_string(&path).unwrap_or_else(|_| {
        panic!("test fixture not found: {path}. Run the download script first.")
    })
}

/// Start a wiremock server that serves `fixture` (a `data/tests/*.json` file)
/// for `GET <path_seg>`, standing in for a real provider endpoint.
async fn mock_provider(path_seg: &str, fixture: &str) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(path_seg))
        .respond_with(ResponseTemplate::new(200).set_body_string(load_fixture(fixture)))
        .mount(&server)
        .await;
    server
}

#[test]
fn id_is_stable_and_provider_scoped() {
    let a = make_id(DiscoverProvider::Itunes, "https://f.example/feed.xml");
    let b = make_id(DiscoverProvider::Itunes, "https://f.example/feed.xml");
    let c = make_id(DiscoverProvider::Gpodder, "https://f.example/feed.xml");
    assert_eq!(a, b, "same provider+feed must hash identically");
    assert_ne!(a, c, "different providers must not collide");
    assert!(a.starts_with("itunes-"));
}

#[test]
fn clamp_keeps_short_strings_and_bounds_long_ones() {
    assert_eq!(clamp_description("hi".into()), "hi");
    let long = "x".repeat(MAX_DESCRIPTION + 100);
    let out = clamp_description(long);
    assert!(out.chars().count() <= MAX_DESCRIPTION + 1); // +1 for the ellipsis
    assert!(out.ends_with('…'));
}

#[test]
fn providers_info_lists_all_available() {
    // Bases are irrelevant here — `providers_info` lists static metadata.
    let svc = DiscoverService::with_bases(String::new(), String::new());
    let info = svc.providers_info();
    assert_eq!(info.providers.len(), PROVIDERS.len());
    assert!(
        info.providers
            .iter()
            .all(|p| p.available && p.default_enabled)
    );
}

#[tokio::test]
async fn itunes_parses_fixture() {
    let server = mock_provider("/search", "discover_itunes_search.json").await;
    let client = reqwest::Client::new();
    let base = format!("{}/search", server.uri());
    let items = itunes::search_at(&client, &base, "rust")
        .await
        .expect("itunes parses");

    assert_eq!(items.len(), 3);
    assert!(items.iter().all(|i| i.provider == DiscoverProvider::Itunes));
    assert!(items.iter().all(|i| i.id.starts_with("itunes-")));
    // iTunes returns no description.
    assert!(items.iter().all(|i| i.description.is_empty()));
    let first = &items[0];
    assert_eq!(first.title, "Learning Rust For Busy People");
    assert_eq!(first.author.as_deref(), Some("Josh Reed"));
    assert_eq!(
        first.feed_url,
        "https://api.substack.com/feed/podcast/7790682.rss"
    );
}

#[tokio::test]
async fn gpodder_parses_fixture() {
    let server = mock_provider("/search.json", "discover_gpodder_search.json").await;
    let client = reqwest::Client::new();
    let base = format!("{}/search.json", server.uri());
    let items = gpodder::search_at(&client, &base, "rust")
        .await
        .expect("gpodder parses");

    assert_eq!(items.len(), 3);
    assert!(
        items
            .iter()
            .all(|i| i.provider == DiscoverProvider::Gpodder)
    );
    let first = &items[0];
    assert_eq!(first.title, "Hörbar Rust");
    assert_eq!(
        first.feed_url,
        "http://www.radioeins.de/archiv/podcast/hoerbar_rust.feed.podcast.xml"
    );
    assert!(!first.description.is_empty());
}

#[tokio::test]
async fn search_is_fault_tolerant_when_one_provider_fails() {
    let itunes = mock_provider("/search", "discover_itunes_search.json").await;
    // gpodder returns 500 → becomes a per-provider error, must NOT blank the search.
    let gpodder = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/search.json"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&gpodder)
        .await;

    let svc = DiscoverService::with_bases(
        format!("{}/search", itunes.uri()),
        format!("{}/search.json", gpodder.uri()),
    );
    let data = svc.search("rust", None).await;

    assert_eq!(
        data.items.len(),
        3,
        "iTunes results survive a gpodder failure"
    );
    assert!(
        data.items
            .iter()
            .all(|i| i.provider == DiscoverProvider::Itunes)
    );
    assert_eq!(data.errors.len(), 1);
    assert_eq!(data.errors[0].provider, DiscoverProvider::Gpodder);
}

#[tokio::test]
async fn search_filter_queries_only_selected_provider() {
    let itunes = mock_provider("/search", "discover_itunes_search.json").await;
    let gpodder = MockServer::start().await; // no mock mounted — must never be hit

    let svc = DiscoverService::with_bases(
        format!("{}/search", itunes.uri()),
        format!("{}/search.json", gpodder.uri()),
    );
    let data = svc.search("rust", Some(&[DiscoverProvider::Itunes])).await;

    assert!(data.errors.is_empty());
    assert_eq!(data.items.len(), 3);
    assert!(
        gpodder.received_requests().await.unwrap().is_empty(),
        "filtered-out provider must not be queried"
    );
}

#[tokio::test]
async fn episode_mapping_partial_errors_and_filter() {
    use wiremock::matchers::query_param;
    let server = MockServer::start().await;
    Mock::given(path("/search"))
        .and(query_param("entity", "podcastEpisode"))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"results":[{"kind":"podcast-episode","trackId":12,"trackName":"Episode","collectionName":"Show","feedUrl":"https://example.com/feed","episodeGuid":"guid","description":"Full text","releaseDate":"2026-09-08T00:00:00Z","trackTimeMillis":125000},{"kind":"podcast","trackName":"Wrong kind"}]}"#))
        .expect(2)
        .mount(&server).await;
    let svc = DiscoverService::with_bases(format!("{}/search", server.uri()), String::new());
    let result = svc.search_episodes("episode", None).await;
    assert_eq!(result.items.len(), 1);
    assert_eq!(result.items[0].duration_seconds, Some(125));
    assert_eq!(result.items[0].podcast_title, "Show");
    assert_eq!(result.errors[0].provider, DiscoverProvider::Gpodder);
    let result = svc
        .search_episodes("episode", Some(&[DiscoverProvider::Itunes]))
        .await;
    assert!(result.errors.is_empty());
    let result = svc
        .search_episodes("episode", Some(&[DiscoverProvider::Gpodder]))
        .await;
    assert!(result.items.is_empty());
    assert_eq!(result.errors.len(), 1);
}
