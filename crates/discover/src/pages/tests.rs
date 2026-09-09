use super::*;
use serde_json::json;
use std::collections::HashSet;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{path, query_param},
};

fn params(cursor: Option<String>) -> DiscoverPageParams {
    DiscoverPageParams {
        q: "Meditation".into(),
        providers: Some(vec![DiscoverProvider::Itunes]),
        cursor,
    }
}

async fn service(episodes: bool) -> DiscoverService {
    let server = MockServer::start().await;
    let rows: Vec<_> = (0..61).map(|i| if episodes {
        json!({"kind":"podcast-episode","trackId":i,"trackName":format!("Meditation {i}"),"collectionName":format!("Show {}", i % 3),"feedUrl":format!("https://example.com/feed/{}", i % 3),"episodeGuid":format!("episode-{i}")})
    } else {
        json!({"collectionName":format!("Show {i}"),"feedUrl":format!("https://example.com/feed/{i}")})
    }).collect();
    Mock::given(path("/search"))
        .and(query_param("limit", "200"))
        .and(query_param("term", "Meditation"))
        .and(query_param(
            "entity",
            if episodes {
                "podcastEpisode"
            } else {
                "podcast"
            },
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"results":rows})))
        .expect(1)
        .mount(&server)
        .await;
    let service = DiscoverService::with_bases(
        format!("{}/search", server.uri()),
        format!("{}/unused", server.uri()),
    );
    // Fetch while the mock is alive; every continuation must use the stored snapshot.
    if episodes {
        service.episode_page(params(None)).await.unwrap();
    } else {
        service.podcast_page(params(None)).await.unwrap();
    }
    service
}

#[tokio::test]
async fn global_episode_snapshot_pages_retry_without_refetch() {
    let service = service(true).await;
    let cursor = {
        let cache = service.pages.lock().unwrap();
        cache.snapshots[0].page(0).unwrap().2.next_cursor.unwrap()
    };
    let second = service
        .episode_page(params(Some(cursor.clone())))
        .await
        .unwrap();
    assert_eq!(second.items.len(), 25);
    assert_eq!(second.page.result_limit, 200);
    assert!(
        second
            .items
            .iter()
            .map(|i| &i.podcast_title)
            .collect::<HashSet<_>>()
            .len()
            > 1
    );
    assert_eq!(
        second,
        service
            .episode_page(params(Some(cursor.clone())))
            .await
            .unwrap()
    );
    let last = service
        .episode_page(params(second.page.next_cursor.clone()))
        .await
        .unwrap();
    assert_eq!(last.items.len(), 11);
    assert!(!last.page.has_more);
    assert!(last.page.next_cursor.is_none());
    let mut changed = params(Some(cursor.clone()));
    changed.q = "different".into();
    assert!(service.episode_page(changed).await.is_err());
    let mut changed = params(Some(cursor.clone()));
    changed.providers = Some(vec![DiscoverProvider::Gpodder]);
    assert!(service.episode_page(changed).await.is_err());
    assert!(
        service
            .podcast_page(params(Some(cursor.clone())))
            .await
            .is_err()
    );
    let mut same = params(Some(cursor));
    same.q = "  Meditation  ".into();
    assert_eq!(second, service.episode_page(same).await.unwrap());
}

#[tokio::test]
async fn podcast_snapshot_expiry_and_malformed_cursors_fail_closed() {
    let service = service(false).await;
    let cursor = {
        let cache = service.pages.lock().unwrap();
        cache.snapshots[0].page(0).unwrap().2.next_cursor.unwrap()
    };
    assert_eq!(
        service
            .podcast_page(params(Some(cursor.clone())))
            .await
            .unwrap()
            .items
            .len(),
        25
    );
    for value in [
        "garbage".into(),
        "x".repeat(100),
        cursor.replace(":25", ":26"),
        cursor.replace(":25", ":0"),
        cursor.replace(":25", ":999999999999999999999"),
    ] {
        assert!(service.podcast_page(params(Some(value))).await.is_err());
    }
    service.pages.lock().unwrap().snapshots[0].created = Instant::now() - SNAPSHOT_TTL;
    assert!(
        service
            .podcast_page(params(Some(cursor)))
            .await
            .unwrap_err()
            .contains("restart")
    );
}

#[tokio::test]
async fn podcast_cross_provider_dedup_and_partial_errors() {
    let server = MockServer::start().await;
    Mock::given(path("/itunes"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"results":[{"collectionName":"One","feedUrl":"https://example.com/feed"}]}),
        ))
        .mount(&server)
        .await;
    Mock::given(path("/gpodder"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!([{"title":"Same show","url":"https://example.com/feed"}])),
        )
        .mount(&server)
        .await;
    let service = DiscoverService::with_bases(
        format!("{}/itunes", server.uri()),
        format!("{}/gpodder", server.uri()),
    );
    let data = service
        .podcast_page(DiscoverPageParams {
            providers: None,
            ..params(None)
        })
        .await
        .unwrap();
    assert_eq!(data.items.len(), 1);
    assert_eq!(data.page.result_limit, 220);
    assert!(data.errors.is_empty());
    let broken = DiscoverService::with_bases(
        format!("{}/itunes", server.uri()),
        format!("{}/failure", server.uri()),
    );
    let data = broken
        .podcast_page(DiscoverPageParams {
            providers: None,
            ..params(None)
        })
        .await
        .unwrap();
    assert_eq!(data.items.len(), 1);
    assert_eq!(data.errors.len(), 1);
    assert!(!data.page.has_more);
}

#[tokio::test]
async fn snapshot_count_is_bounded_and_old_cursor_is_evicted() {
    let server = MockServer::start().await;
    let rows: Vec<_> = (0..26).map(|i| json!({"collectionName":format!("Show {i}"),"feedUrl":format!("https://example.com/feed/{i}")})).collect();
    Mock::given(path("/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"results":rows})))
        .expect(33)
        .mount(&server)
        .await;
    let service = DiscoverService::with_bases(format!("{}/search", server.uri()), String::new());
    let first = service.podcast_page(params(None)).await.unwrap();
    for _ in 0..32 {
        service.podcast_page(params(None)).await.unwrap();
    }
    assert_eq!(service.pages.lock().unwrap().snapshots.len(), MAX_SNAPSHOTS);
    assert!(
        service
            .podcast_page(params(first.page.next_cursor))
            .await
            .is_err()
    );
}
