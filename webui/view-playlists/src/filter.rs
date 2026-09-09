//! Pure client-side search + sort for the playlists list. Playlist sets are small and held in the offline-first pool,
//! so we filter + sort in memory for instant feedback (the server fetch fills in more matches as you scroll). Sibling
//! of `pages::podcasts::filter::apply_podcast_search_sort`; the in-memory mirror of the server's `order_by`/direction
//! mapping in `pages::playlists` (`order_by_token`/`db_direction`).

use halogen_wire::PlaylistData;

use crate::components::{FilterSpec, SortField, SortSpec, search_sort};

/// Filters by `filter.search` (case-insensitive `contains` on the name) and sorts
/// by `sort`: Custom→`position`, CreatedAt→`created_at`, anything else→name
/// (case-insensitive), each tie-broken by `id`, then reversed for `Desc`.
pub fn apply_playlist_search_sort(
    rows: Vec<PlaylistData>,
    sort: &SortSpec,
    filter: &FilterSpec,
) -> Vec<PlaylistData> {
    let q = filter
        .search
        .as_ref()
        .map(|s| s.to_lowercase())
        .unwrap_or_default();
    search_sort(
        rows,
        sort.direction,
        |pl| q.is_empty() || pl.name.to_lowercase().contains(&q),
        |a, b| match sort.field {
            SortField::Custom => a.position.cmp(&b.position).then(a.id.cmp(&b.id)),
            SortField::CreatedAt => a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)),
            _ => a
                .name
                .to_lowercase()
                .cmp(&b.name.to_lowercase())
                .then(a.id.cmp(&b.id)),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::OrderDirection;
    use chrono::{TimeZone, Utc};

    fn playlist(id: i32, name: &str, position: i32, day: u32) -> PlaylistData {
        PlaylistData {
            id,
            name: name.into(),
            description: None,
            is_default: false,
            position,
            on_remove_delete_file_server: false,
            on_remove_delete_file_client: false,
            created_at: Utc.with_ymd_and_hms(2026, 1, day, 0, 0, 0).unwrap(),
            updated_at: Utc.with_ymd_and_hms(2026, 1, day, 0, 0, 0).unwrap(),
            episode_ids: None,
            episode_playlist: None,
        }
    }

    fn ids(rows: &[PlaylistData]) -> Vec<i32> {
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

    fn rows() -> Vec<PlaylistData> {
        vec![
            // id, name, position, created-day.
            playlist(1, "Cooking", 2, 3),
            playlist(2, "rust weekly", 0, 1),
            playlist(3, "ana's mix", 1, 2),
        ]
    }

    #[test]
    fn search_matches_name_case_insensitive() {
        let got = apply_playlist_search_sort(
            rows(),
            &sort(SortField::Custom, OrderDirection::Asc),
            &search("RUST"),
        );
        assert_eq!(ids(&got), vec![2]);
    }

    #[test]
    fn empty_search_keeps_all() {
        let got = apply_playlist_search_sort(
            rows(),
            &sort(SortField::Custom, OrderDirection::Asc),
            &FilterSpec::default(),
        );
        assert_eq!(got.len(), 3);
    }

    #[test]
    fn sort_by_custom_position_asc_desc() {
        // positions: id2=0, id3=1, id1=2.
        let asc = apply_playlist_search_sort(
            rows(),
            &sort(SortField::Custom, OrderDirection::Asc),
            &FilterSpec::default(),
        );
        assert_eq!(ids(&asc), vec![2, 3, 1]);

        let desc = apply_playlist_search_sort(
            rows(),
            &sort(SortField::Custom, OrderDirection::Desc),
            &FilterSpec::default(),
        );
        assert_eq!(ids(&desc), vec![1, 3, 2]);
    }

    #[test]
    fn sort_by_created_at_asc_desc() {
        // created days: id2=Jan1, id3=Jan2, id1=Jan3.
        let asc = apply_playlist_search_sort(
            rows(),
            &sort(SortField::CreatedAt, OrderDirection::Asc),
            &FilterSpec::default(),
        );
        assert_eq!(ids(&asc), vec![2, 3, 1]);

        let desc = apply_playlist_search_sort(
            rows(),
            &sort(SortField::CreatedAt, OrderDirection::Desc),
            &FilterSpec::default(),
        );
        assert_eq!(ids(&desc), vec![1, 3, 2]);
    }

    #[test]
    fn sort_by_name_is_case_insensitive_asc_desc() {
        // Names lowercased: "ana's mix" < "cooking" < "rust weekly". Any non-Custom,
        // non-CreatedAt field falls through to the name comparator.
        let asc = apply_playlist_search_sort(
            rows(),
            &sort(SortField::Title, OrderDirection::Asc),
            &FilterSpec::default(),
        );
        assert_eq!(ids(&asc), vec![3, 1, 2]);

        let desc = apply_playlist_search_sort(
            rows(),
            &sort(SortField::Title, OrderDirection::Desc),
            &FilterSpec::default(),
        );
        assert_eq!(ids(&desc), vec![2, 1, 3]);
    }
}
