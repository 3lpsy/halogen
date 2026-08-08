//! Pure client-side search + sort for the podcasts list.
//!
//! Podcast sets are small and held fully in the offline-first pool
//! (`EpisodeState::podcasts_by_id`), so we filter + sort in memory rather than
//! round-tripping the server. Mirrors `episode_list::list::apply_filter_sort`,
//! but only the two facets that make sense for podcasts: a title/author search
//! and a Title / Added sort. The episode-only `FilterSpec` fields (chips,
//! `podcast_id`, …) are intentionally ignored.

use halogen_wire::PodcastData;

use crate::components::{FilterSpec, SortField, SortSpec, search_sort};

/// Filter `rows` by `filter.search` (case-insensitive contains on title +
/// author) and sort by `sort` (Title or Added; anything else falls back to id).
pub fn apply_podcast_search_sort(
    rows: Vec<PodcastData>,
    sort: &SortSpec,
    filter: &FilterSpec,
) -> Vec<PodcastData> {
    let query = filter
        .search
        .as_ref()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_lowercase());
    search_sort(
        rows,
        sort.direction,
        |p| match &query {
            None => true,
            Some(q) => {
                p.title.to_lowercase().contains(q)
                    || p.author
                        .as_ref()
                        .is_some_and(|a| a.to_lowercase().contains(q))
            }
        },
        |a, b| match sort.field {
            SortField::Title => a
                .title
                .to_lowercase()
                .cmp(&b.title.to_lowercase())
                .then_with(|| a.id.cmp(&b.id)),
            SortField::CreatedAt => a
                .created_at
                .cmp(&b.created_at)
                .then_with(|| a.id.cmp(&b.id)),
            // Title / Added are the only fields the controls offer; anything else
            // (restored from a shared URL for another list) falls back to id.
            _ => a.id.cmp(&b.id),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::OrderDirection;
    use chrono::{TimeZone, Utc};

    fn podcast(id: i32, title: &str, author: Option<&str>, day: u32) -> PodcastData {
        PodcastData {
            id,
            title: title.into(),
            description: String::new(),
            feed_url: format!("https://feed.test/{id}"),
            art_url: None,
            author: author.map(|a| a.into()),
            polled_at: None,
            podcast_config_id: None,
            art_file_path: None,
            etag: None,
            last_modified: None,
            podcast_config: None,
            created_at: Utc.with_ymd_and_hms(2026, 1, day, 0, 0, 0).unwrap(),
            updated_at: Utc.with_ymd_and_hms(2026, 1, day, 0, 0, 0).unwrap(),
            episode_count: None,
            feed_url_redirects: None,
        }
    }

    fn ids(rows: &[PodcastData]) -> Vec<i32> {
        rows.iter().map(|p| p.id).collect()
    }

    fn sort(field: SortField, direction: OrderDirection) -> SortSpec {
        SortSpec { field, direction }
    }

    fn search(q: &str) -> FilterSpec {
        FilterSpec {
            search: Some(q.into()),
            ..Default::default()
        }
    }

    fn rows() -> Vec<PodcastData> {
        vec![
            podcast(1, "Cooking Show", Some("Jane Doe"), 3),
            podcast(2, "Rust Weekly", Some("Ferris"), 1),
            podcast(3, "ana's banana hour", None, 2),
        ]
    }

    #[test]
    fn search_matches_title() {
        let got = apply_podcast_search_sort(
            rows(),
            &sort(SortField::Title, OrderDirection::Asc),
            &search("rust"),
        );
        assert_eq!(ids(&got), vec![2]);
    }

    #[test]
    fn search_matches_author() {
        let got = apply_podcast_search_sort(
            rows(),
            &sort(SortField::Title, OrderDirection::Asc),
            &search("ferris"),
        );
        assert_eq!(ids(&got), vec![2]);
    }

    #[test]
    fn search_is_case_insensitive() {
        let got = apply_podcast_search_sort(
            rows(),
            &sort(SortField::Title, OrderDirection::Asc),
            &search("COOKING"),
        );
        assert_eq!(ids(&got), vec![1]);
    }

    #[test]
    fn empty_search_keeps_all() {
        let got = apply_podcast_search_sort(
            rows(),
            &sort(SortField::Title, OrderDirection::Asc),
            &FilterSpec::default(),
        );
        assert_eq!(got.len(), 3);
    }

    #[test]
    fn sort_by_title_is_case_insensitive_asc_desc() {
        // "ana's banana hour" sorts first case-insensitively (would sort last if
        // raw ASCII, since lowercase 'a' > uppercase 'C'/'R').
        let asc = apply_podcast_search_sort(
            rows(),
            &sort(SortField::Title, OrderDirection::Asc),
            &FilterSpec::default(),
        );
        assert_eq!(ids(&asc), vec![3, 1, 2]);

        let desc = apply_podcast_search_sort(
            rows(),
            &sort(SortField::Title, OrderDirection::Desc),
            &FilterSpec::default(),
        );
        assert_eq!(ids(&desc), vec![2, 1, 3]);
    }

    #[test]
    fn sort_by_added_asc_desc() {
        // created_at days: id2=Jan1, id3=Jan2, id1=Jan3.
        let asc = apply_podcast_search_sort(
            rows(),
            &sort(SortField::CreatedAt, OrderDirection::Asc),
            &FilterSpec::default(),
        );
        assert_eq!(ids(&asc), vec![2, 3, 1]);

        let desc = apply_podcast_search_sort(
            rows(),
            &sort(SortField::CreatedAt, OrderDirection::Desc),
            &FilterSpec::default(),
        );
        assert_eq!(ids(&desc), vec![1, 3, 2]);
    }
}
