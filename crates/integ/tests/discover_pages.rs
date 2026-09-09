use halogen_integ::*;
use halogen_wire::{DiscoverPageParams, DiscoverPodcastParams, DiscoverProvider};
use serde_json::json;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn params(cursor: Option<String>) -> DiscoverPageParams {
    DiscoverPageParams {
        q: "Meditation".into(),
        providers: Some(vec![DiscoverProvider::Itunes]),
        cursor,
    }
}

#[tokio::test]
async fn discover_pages_are_authenticated_and_cross_podcast_with_stable_retries() {
    let provider = MockServer::start().await;
    let rows: Vec<_> = (0..30).map(|index| json!({
        "kind": "podcast-episode", "trackId": index, "trackName": format!("Meditation {index}"),
        "collectionName": format!("Show {}", index % 3),
        "feedUrl": format!("https://example.invalid/{}.rss", index % 3),
        "episodeGuid": format!("episode-{index}")
    })).collect();
    Mock::given(method("GET"))
        .and(path("/search"))
        .and(query_param("entity", "podcastEpisode"))
        .and(query_param("term", "Meditation"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"results": rows})))
        .expect(1)
        .mount(&provider)
        .await;
    let app = spawn_with(SpawnOptions {
        discover_itunes_base_url: Some(format!("{}/search", provider.uri())),
        ..Default::default()
    })
    .await;
    let anonymous = anon_api(&app)
        .discover_episode_page(params(None))
        .await
        .unwrap_err();
    assert_eq!(status_of(&anonymous), 401);
    let admin = app.seed_admin().await;
    let client = api(&app, &admin.token);
    let first = client.discover_episode_page(params(None)).await.unwrap();
    assert_eq!(first.items.len(), 25);
    assert_eq!(
        first
            .items
            .iter()
            .map(|item| &item.podcast_title)
            .collect::<std::collections::HashSet<_>>()
            .len(),
        3
    );
    let next = params(first.page.next_cursor);
    let second = client.discover_episode_page(next.clone()).await.unwrap();
    assert_eq!(second.items.len(), 5);
    assert!(!second.page.has_more);
    assert_eq!(
        client.discover_episode_page(next).await.unwrap().items,
        second.items
    );
    let mut invalid = params(None);
    invalid.q = "x".repeat(257);
    assert_eq!(
        status_of(&client.discover_episode_page(invalid).await.unwrap_err()),
        400
    );
}

#[tokio::test]
async fn discover_preview_uses_only_mock_feed_and_requires_auth() {
    let feeds = MockServer::start().await;
    Mock::given(method("GET")).and(path("/podcast.xml"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            "<rss version=\"2.0\"><channel><title>Mock show</title><link>https://example.invalid</link><description>Podcast description</description><item><title>Meditation</title><guid>meditation</guid><description>Episode description</description></item></channel></rss>"
        )).expect(1).mount(&feeds).await;
    let app = spawn_with(SpawnOptions::default()).await;
    let params = DiscoverPodcastParams {
        feed_url: format!("{}/podcast.xml", feeds.uri()),
        provider: DiscoverProvider::Itunes,
    };
    assert_eq!(
        status_of(
            &anon_api(&app)
                .discover_podcast(params.clone())
                .await
                .unwrap_err()
        ),
        401
    );
    let admin = app.seed_admin().await;
    let data = api(&app, &admin.token)
        .discover_podcast(params)
        .await
        .unwrap();
    assert_eq!(data.podcast.title, "Mock show");
    assert_eq!(data.podcast.description, "Podcast description");
    assert_eq!(data.episodes[0].title, "Meditation");
    assert_eq!(data.episodes[0].description, "Episode description");
}
