use super::*;

#[test]
fn feed_preserves_description_and_scopes_episode_ids() {
    let description = "complete description ".repeat(200);
    let xml = format!(
        r#"<rss version="2.0" xmlns:itunes="http://www.itunes.com/dtds/podcast-1.0.dtd"><channel><title>Show</title><link>https://example.com</link><description>Podcast</description><item><title>Episode</title><guid>episode-one</guid><description>{description}</description><itunes:duration>1:02:03</itunes:duration><pubDate>Tue, 08 Sep 2026 12:00:00 +0000</pubDate></item></channel></rss>"#
    );
    let result = parse_feed(
        xml.as_bytes(),
        "https://example.com/feed",
        DiscoverProvider::Itunes,
    )
    .unwrap();
    assert_eq!(result.episodes.len(), 1);
    assert_eq!(result.episodes[0].description, description.trim());
    assert_eq!(result.episodes[0].duration_seconds, Some(3723));
    assert_ne!(result.podcast.id, result.episodes[0].id);
    assert!(result.episodes[0].published_at.is_some());
}

#[tokio::test]
async fn feed_rejects_large_and_invalid_responses() {
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path};
    let server = MockServer::start().await;
    Mock::given(path("/large"))
        .respond_with(ResponseTemplate::new(200).set_body_string("x".repeat(16 * 1024 * 1024 + 1)))
        .mount(&server)
        .await;
    let svc = DiscoverService::with_bases(String::new(), String::new());
    let error = svc
        .podcast_preview(&format!("{}/large", server.uri()), DiscoverProvider::Itunes)
        .await
        .unwrap_err();
    assert!(error.contains("safety limit"));
    assert!(
        svc.podcast_preview("file:///etc/passwd", DiscoverProvider::Itunes)
            .await
            .is_err()
    );
    assert!(
        svc.podcast_preview(
            "https://user:pass@example.com/feed",
            DiscoverProvider::Itunes
        )
        .await
        .is_err()
    );
    assert!(parse_feed(b"not rss", "https://example.com", DiscoverProvider::Itunes).is_err());
}

#[test]
fn oversized_repeated_fields_are_bounded() {
    let title = "t".repeat(10000);
    let description = "d".repeat(70000);
    let item = format!(
        "<item><title>{title}</title><guid>one</guid><description>{description}</description></item>"
    );
    let xml = format!(
        "<rss version=\"2.0\"><channel><title>{title}</title><link>https://example.com</link><description>{description}</description>{item}</channel></rss>"
    );
    let result = parse_feed(
        xml.as_bytes(),
        "https://example.com/feed",
        DiscoverProvider::Itunes,
    )
    .unwrap();
    assert_eq!(result.podcast.title.len(), 512);
    assert_eq!(result.podcast.description.len(), 65536);
    assert_eq!(result.episodes[0].podcast_title.len(), 512);
    assert_eq!(result.episodes[0].title.len(), 512);
    assert_eq!(result.episodes[0].description.len(), 65536);
}

#[tokio::test]
async fn large_archive_returns_description_and_first_200_episodes() {
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path};
    let server = MockServer::start().await;
    let mut xml = String::from(
        r#"<rss version="2.0" xmlns:itunes="http://www.itunes.com/dtds/podcast-1.0.dtd"><channel><title>Archive</title><description><![CDATA[<p>About this podcast</p>]]></description><itunes:author>Author</itunes:author>"#,
    );
    for id in 0..1200 {
        xml.push_str(&format!("<item><title>Episode {id}</title><guid>{id}</guid><description>{}</description><itunes:duration>2:30</itunes:duration></item>", "a".repeat(4096)));
    }
    xml.push_str("</channel></rss>");
    assert!(xml.len() > 4 * 1024 * 1024);
    Mock::given(path("/archive"))
        .respond_with(ResponseTemplate::new(200).set_body_string(xml))
        .expect(1)
        .mount(&server)
        .await;
    let service = DiscoverService::with_bases(String::new(), String::new());
    let preview = service
        .podcast_preview(
            &format!("{}/archive", server.uri()),
            DiscoverProvider::Itunes,
        )
        .await
        .unwrap();
    assert_eq!(preview.podcast.description, "<p>About this podcast</p>");
    assert_eq!(preview.podcast.author.as_deref(), Some("Author"));
    assert_eq!(preview.episodes.len(), 200);
    assert_eq!(preview.episodes[199].title, "Episode 199");
    assert_eq!(preview.episodes[199].duration_seconds, Some(150));
}
