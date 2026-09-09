use super::*;

#[test]
fn blank_description_is_an_explicit_valid_update() {
    let patch = update_data("Podcast", "  \n ", " https://example.test/feed.xml ");
    assert_eq!(patch.description.as_deref(), Some(""));
    assert_eq!(
        patch.feed_url.as_deref(),
        Some("https://example.test/feed.xml")
    );
    assert!(patch.validate().is_ok());
}
