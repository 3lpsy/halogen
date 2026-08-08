use crate::feed::*;
use crate::types::{RemoteChapter, RemoteEpisodeData};
use chrono::{DateTime, Utc};
use halogen_fixture::test_support::load_fixture;

// Channel-level artwork is the primary podcast-art source (98/98 sampled
// feeds carry it; episode-level images are spotty).
#[test]
fn test_parse_feed_extracts_channel_art() {
    let xml = load_fixture("simplecast_the_daily.xml");
    let feed = parse_feed(&xml).expect("should parse");
    let art = feed.art_url.expect("channel art present");
    assert!(art.starts_with("https://"), "channel art is a URL: {art}");
    assert!(!feed.episodes.is_empty());
}

#[test]
fn test_parse_simplecast_feed() {
    let xml = load_fixture("simplecast_the_daily.xml");
    let episodes = parse_rss(&xml).expect("should parse simplecast feed");
    assert!(!episodes.is_empty(), "simplecast feed should have episodes");
    for ep in &episodes {
        assert!(!ep.title.is_empty(), "episode should have a title");
        assert!(
            !ep.content_url.is_empty(),
            "episode should have a content_url"
        );
    }
}

#[test]
fn test_parse_npt_seconds() {
    assert_eq!(parse_npt_seconds("3600"), Some(3600)); // plain seconds
    assert_eq!(parse_npt_seconds("62:03"), Some(3723)); // MM:SS
    assert_eq!(parse_npt_seconds("1:02:03"), Some(3723)); // HH:MM:SS
    assert_eq!(parse_npt_seconds("0:30"), Some(30));
    assert_eq!(parse_npt_seconds("00:13:39"), Some(819)); // psc chapter start
    assert_eq!(parse_npt_seconds("839.5"), Some(839)); // fractional seconds truncated
    assert_eq!(parse_npt_seconds("00:13:39.500"), Some(819)); // fractional HH:MM:SS
    assert_eq!(parse_npt_seconds(""), None);
    assert_eq!(parse_npt_seconds("  "), None);
    assert_eq!(parse_npt_seconds("garbage"), None);
    assert_eq!(parse_npt_seconds("1:bad:3"), None);
}

#[test]
fn test_parse_transistor_feed() {
    let xml = load_fixture("transistor_acquired.xml");
    let episodes = parse_rss(&xml).expect("should parse transistor feed");
    assert!(!episodes.is_empty(), "transistor feed should have episodes");
}

// `transistor_acquired.xml` carries `<podcast:chapters url=… type=json+chapters>`
// on at least one item — confirm we surface the external URL (and no inline set).
#[test]
fn test_parse_podcast_chapters_url() {
    let xml = load_fixture("transistor_acquired.xml");
    let episodes = parse_rss(&xml).expect("should parse transistor feed");
    let with_url = episodes
        .iter()
        .find(|e| e.chapters_url.is_some())
        .expect("a transistor item declares podcast:chapters url");
    assert!(
        with_url
            .chapters_url
            .as_deref()
            .is_some_and(|u| u.starts_with("https://") && u.ends_with(".json")),
        "chapters_url is the external JSON: {:?}",
        with_url.chapters_url
    );
    assert!(
        with_url.chapters.is_empty(),
        "external-URL items carry no inline chapters"
    );
}

// `omny_odd_lots.xml` carries inline `<psc:chapters>` — confirm we parse the
// markers and convert `start` (HH:MM:SS) to whole seconds in order.
#[test]
fn test_parse_psc_inline_chapters() {
    let xml = load_fixture("omny_odd_lots.xml");
    let episodes = parse_rss(&xml).expect("should parse omny feed");
    let with_ch = episodes
        .iter()
        .find(|e| !e.chapters.is_empty())
        .expect("an omny item declares inline psc:chapters");
    // First marker is always 00:00:00 in the fixtures.
    assert_eq!(with_ch.chapters[0].starts_at_secs, 0);
    assert!(
        !with_ch.chapters[0].title.is_empty(),
        "chapter carries a title"
    );
    // Non-decreasing start offsets (feed order preserved).
    assert!(
        with_ch
            .chapters
            .windows(2)
            .all(|w| w[0].starts_at_secs <= w[1].starts_at_secs),
        "chapters are in start order"
    );
    // An item with no chapters element surfaces an empty vec (never panics).
    assert!(episodes.iter().any(|e| e.chapters.is_empty()));
}

#[test]
fn test_parse_npr_feed() {
    let xml = load_fixture("npr_embedded.xml");
    let episodes = parse_rss(&xml).expect("should parse NPR feed");
    assert!(!episodes.is_empty(), "NPR feed should have episodes");
}

#[test]
fn test_parse_syntax_feed() {
    let xml = load_fixture("syntax_fm.xml");
    let episodes = parse_rss(&xml).expect("should parse syntax.fm feed");
    assert!(!episodes.is_empty(), "syntax.fm feed should have episodes");
}

#[test]
fn test_parse_audioboom_feed() {
    let xml = load_fixture("audioboom_no_such_thing.xml");
    let episodes = parse_rss(&xml).expect("should parse audioboom feed");
    assert!(!episodes.is_empty(), "audioboom feed should have episodes");
}

#[test]
fn test_parse_sed_feed() {
    let xml = load_fixture("sed_podcast.xml");
    let episodes = parse_rss(&xml).expect("should parse SED feed");
    assert!(!episodes.is_empty(), "SED feed should have episodes");
}

#[test]
fn test_parse_hanselminutes_feed() {
    let xml = load_fixture("hanselminutes.xml");
    let episodes = parse_rss(&xml).expect("should parse hanselminutes feed");
    assert!(
        !episodes.is_empty(),
        "hanselminutes feed should have episodes"
    );
}

#[test]
fn test_parse_devotea_feed() {
    let xml = load_fixture("devotea.xml");
    let episodes = parse_rss(&xml).expect("should parse devotea feed");
    assert!(!episodes.is_empty(), "devotea feed should have episodes");
}

#[test]
fn test_parse_99invisible_feed() {
    let xml = load_fixture("simplecast_99invisible.xml");
    let episodes = parse_rss(&xml).expect("should parse 99% invisible feed");
    assert!(
        !episodes.is_empty(),
        "99% invisible feed should have episodes"
    );
}

#[test]
fn test_remote_episode_serialization() {
    let ep = RemoteEpisodeData {
        title: "Test Episode".to_string(),
        description: Some("A test description".to_string()),
        content_url: "https://example.com/audio.mp3".to_string(),
        art_url: Some("https://example.com/art.jpg".to_string()),
        published_at: Some(
            DateTime::parse_from_rfc3339("2024-01-15T12:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
        ),
        duration_secs: Some(1830),
        guid: Some("guid-123".to_string()),
        chapters: vec![RemoteChapter {
            title: "Intro".to_string(),
            starts_at_secs: 0,
        }],
        chapters_url: None,
    };

    let json = serde_json::to_string(&ep).expect("should serialize");
    let deserialized: RemoteEpisodeData = serde_json::from_str(&json).expect("should deserialize");
    assert_eq!(ep.title, deserialized.title);
    assert_eq!(ep.content_url, deserialized.content_url);
}

#[test]
fn test_parse_empty_feed() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <rss version="2.0">
        <channel>
            <title>Empty Feed</title>
            <link>https://example.com</link>
            <description>An empty feed</description>
        </channel>
    </rss>"#;
    let episodes = parse_rss(xml).expect("should parse empty feed");
    assert!(episodes.is_empty(), "empty feed should have no episodes");
}
