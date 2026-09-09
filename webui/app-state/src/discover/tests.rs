use super::*;

fn page(id: &str, feed: &str) -> DiscoverPodcastPageData {
    DiscoverPodcastPageData {
        items: vec![DiscoverResultItem {
            id: id.into(),
            feed_url: feed.into(),
            provider: DiscoverProvider::Itunes,
            title: "Show".into(),
            description: String::new(),
            author: None,
        }],
        errors: vec![],
        page: DiscoverPageInfo {
            has_more: true,
            next_cursor: Some("cursor".into()),
            result_limit: 200,
        },
    }
}

#[test]
fn changing_search_inputs_discards_old_cursor_and_invalidates_requests() {
    let mut state = DiscoverState::default();
    state.reset(
        "old".into(),
        DiscoverMode::Podcasts,
        vec![DiscoverProvider::Itunes],
    );
    let old_generation = state.generation;
    state.loading = true;
    state.append_podcasts(page("one", "https://example.com/feed"));
    state.reset(
        "Meditation".into(),
        DiscoverMode::Episodes,
        vec![DiscoverProvider::Gpodder],
    );
    assert_ne!(state.generation, old_generation);
    assert!(!state.loading);
    assert!(state.results.is_empty());
    assert!(state.page.is_none());
    assert_eq!(state.providers, vec![DiscoverProvider::Gpodder]);
}

#[test]
fn retried_pages_append_once_and_preserve_first_provider_identity() {
    let mut state = DiscoverState::default();
    state.append_podcasts(page("one", "https://example.com/feed"));
    state.append_podcasts(page("another-provider", "https://example.com/feed"));
    state.append_podcasts(page("two", "https://example.com/other"));
    assert_eq!(state.results.len(), 2);
    assert_eq!(state.results[0].id, "one");
    assert_eq!(state.page.unwrap().next_cursor.as_deref(), Some("cursor"));
}
