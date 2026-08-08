//! The cache-side episode query model: the `EpisodeOrder`/`EpisodeQueryFilter`/
//! `EpisodeQuery` value types plus the shared filter/sort/paginate algorithm the
//! stores run over the cached pool. Kept out of `mod.rs` (the `LocalStore` trait)
//! since these are the request/predicate types, not the trait itself.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use halogen_wire::{DownloadStatus, EpisodeData, PlaybackStatus};

/// Field an episode page is ordered by. Mirrors the server's `order_by` values
/// the client actually uses; the UI `SortField` maps onto this.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EpisodeOrder {
    /// Newest-first feed ordering (the `/latest` default).
    #[default]
    PublishedAt,
    /// Alphabetical by title.
    Title,
    /// Row creation time.
    CreatedAt,
    /// Last-updated time.
    UpdatedAt,
    /// Episode length (`duration_secs`); unknown lengths (`None`) sort as the
    /// smallest value, matching SQL `NULL` ordering on the server.
    Duration,
}

/// Predicate over the cached episode pool — the cache-side mirror of the server
/// `FilterParams`. Empty fields = no constraint. Multi-select facets (playback /
/// download status) are OR within the facet, AND across facets. `search` matches
/// title, description, or the podcast name (resolved via [`Self::podcast_names`]),
/// case-insensitively.
#[derive(Debug, Clone, Default)]
pub struct EpisodeQueryFilter {
    pub search: Option<String>,
    pub podcast_id: Option<i32>,
    pub published_after: Option<DateTime<Utc>>,
    pub playback_status: Vec<PlaybackStatus>,
    pub download_status: Vec<DownloadStatus>,
    /// Restrict to a set of ids (e.g. the device-download set).
    pub ids: Option<HashSet<i32>>,
    /// Podcast id → title, used ONLY so `search` can match the show name. Episode
    /// rows no longer carry the nested podcast join, so the parent name is resolved
    /// from the EpisodeState podcast pool by the caller (see `read_cache`) and populated
    /// here only when a search is active. It's a resolution aid, not a constraint,
    /// so [`Self::is_empty`] ignores it (an empty map never narrows the result).
    pub podcast_names: HashMap<i32, String>,
}

impl EpisodeQueryFilter {
    /// True when nothing is constrained — lets both stores take their fast indexed
    /// `published_at` path (native SQL `LIMIT/OFFSET`, web `idx_pub` cursor) and
    /// O(1) count instead of loading the whole pool.
    pub fn is_empty(&self) -> bool {
        self.search.is_none()
            && self.podcast_id.is_none()
            && self.published_after.is_none()
            && self.playback_status.is_empty()
            && self.download_status.is_empty()
            && self.ids.is_none()
    }

    /// Whether an episode satisfies the predicate.
    pub fn matches(&self, ep: &EpisodeData) -> bool {
        if let Some(s) = &self.search {
            let q = s.to_lowercase();
            let in_title = ep.title.to_lowercase().contains(&q);
            let in_desc = ep
                .description
                .as_ref()
                .is_some_and(|d| d.to_lowercase().contains(&q));
            let in_podcast = self
                .podcast_names
                .get(&ep.podcast_id)
                .is_some_and(|t| t.to_lowercase().contains(&q));
            if !(in_title || in_desc || in_podcast) {
                return false;
            }
        }
        if let Some(pid) = self.podcast_id
            && ep.podcast_id != pid
        {
            return false;
        }
        if let Some(after) = self.published_after {
            match ep.published_at {
                Some(p) if p >= after => {}
                _ => return false,
            }
        }
        if !self.playback_status.is_empty() && !self.playback_status.contains(&ep.playback_status) {
            return false;
        }
        if !self.download_status.is_empty() && !self.download_status.contains(&ep.download_status) {
            return false;
        }
        if let Some(ids) = &self.ids
            && !ids.contains(&ep.id)
        {
            return false;
        }
        true
    }
}

/// A request for one page of episodes from the local cache: filtered, ordered,
/// then sliced the same way the server pages them so the cache view tracks the
/// server's ordering as pages fill the pool.
#[derive(Debug, Clone, Default)]
pub struct EpisodeQuery {
    /// Column to order by.
    pub order_by: EpisodeOrder,
    /// Descending when true (newest/Z-A first).
    pub descending: bool,
    /// 0-based page index.
    pub page: i32,
    /// Page size.
    pub size: i32,
    /// Predicate applied before ordering + slicing.
    pub filter: EpisodeQueryFilter,
}

/// Filter, then sort the matching set the way `EpisodeQuery` asks, then slice the
/// page. Shared by the web store and the native non-fast path so cache ordering
/// matches the server (and the native indexed path). `id` is the deterministic
/// tiebreaker; `None` `published_at` sorts last under DESC.
pub(crate) fn filter_sort_paginate(eps: Vec<EpisodeData>, q: &EpisodeQuery) -> Vec<EpisodeData> {
    let mut eps: Vec<EpisodeData> = eps.into_iter().filter(|e| q.filter.matches(e)).collect();
    eps.sort_by(|a, b| {
        let cmp = match q.order_by {
            EpisodeOrder::PublishedAt => a.published_at.cmp(&b.published_at),
            EpisodeOrder::Title => a.title.cmp(&b.title),
            EpisodeOrder::CreatedAt => a.created_at.cmp(&b.created_at),
            EpisodeOrder::UpdatedAt => a.updated_at.cmp(&b.updated_at),
            EpisodeOrder::Duration => a.duration_secs.cmp(&b.duration_secs),
        }
        .then_with(|| a.id.cmp(&b.id));
        if q.descending { cmp.reverse() } else { cmp }
    });
    let size = q.size.max(1) as usize;
    let start = (q.page.max(0) as usize).saturating_mul(size);
    eps.into_iter().skip(start).take(size).collect()
}

/// Count the episodes matching a filter — drives `has_more` (is there more cached
/// pool to render before fetching the next server page?).
pub(crate) fn filter_count(eps: &[EpisodeData], filter: &EpisodeQueryFilter) -> usize {
    eps.iter().filter(|e| filter.matches(e)).count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use halogen_wire::{DownloadStatus, PlaybackStatus};
    use std::collections::{HashMap, HashSet};

    /// Episode with a given id, title, and optional publish year (None = undated).
    fn ep(id: i32, title: &str, year: Option<i32>) -> EpisodeData {
        let base = Utc.with_ymd_and_hms(2000, 1, 1, 0, 0, 0).unwrap();
        EpisodeData {
            id,
            podcast_id: 1,
            title: title.to_string(),
            description: None,
            content_url: String::new(),
            guid: None,
            art_url: None,
            published_at: year.map(|y| Utc.with_ymd_and_hms(y, 1, 1, 0, 0, 0).unwrap()),
            downloaded_at: None,
            content_file_path: None,
            download_size: None,
            art_file_path: None,
            download_status: DownloadStatus::NotDownloaded,
            download_started_at: None,
            download_attempts: 0,
            playback_status: PlaybackStatus::Unplayed,
            duration_secs: None,
            created_at: base,
            updated_at: base,
            podcast: None,
            playback: None,
            chapters: None,
        }
    }

    fn query(order_by: EpisodeOrder, descending: bool, page: i32, size: i32) -> EpisodeQuery {
        EpisodeQuery {
            order_by,
            descending,
            page,
            size,
            filter: EpisodeQueryFilter::default(),
        }
    }

    fn ids(eps: Vec<EpisodeData>) -> Vec<i32> {
        eps.into_iter().map(|e| e.id).collect()
    }

    #[test]
    fn published_at_desc_newest_first_with_nulls_last() {
        let eps = vec![
            ep(1, "a", Some(2020)),
            ep(2, "b", Some(2023)),
            ep(3, "c", None),
            ep(4, "d", Some(2021)),
        ];
        let out = filter_sort_paginate(eps, &query(EpisodeOrder::PublishedAt, true, 0, 10));
        assert_eq!(ids(out), vec![2, 4, 1, 3], "newest first, undated last");
    }

    #[test]
    fn published_at_asc_oldest_first_with_nulls_first() {
        let eps = vec![
            ep(1, "a", Some(2020)),
            ep(2, "b", None),
            ep(3, "c", Some(2019)),
        ];
        let out = filter_sort_paginate(eps, &query(EpisodeOrder::PublishedAt, false, 0, 10));
        assert_eq!(
            ids(out),
            vec![2, 3, 1],
            "undated first under ASC, then oldest→newest"
        );
    }

    #[test]
    fn duration_order_sorts_by_length_with_unknown_smallest() {
        let with_dur = |id: i32, secs: Option<i32>| {
            let mut e = ep(id, "x", None);
            e.duration_secs = secs;
            e
        };
        let eps = vec![
            with_dur(1, Some(600)),
            with_dur(2, None),
            with_dur(3, Some(60)),
        ];
        let asc = filter_sort_paginate(eps.clone(), &query(EpisodeOrder::Duration, false, 0, 10));
        assert_eq!(
            ids(asc),
            vec![2, 3, 1],
            "unknown first under ASC (like SQL NULL)"
        );
        let desc = filter_sort_paginate(eps, &query(EpisodeOrder::Duration, true, 0, 10));
        assert_eq!(
            ids(desc),
            vec![1, 3, 2],
            "longest first, unknown last under DESC"
        );
    }

    #[test]
    fn title_order_is_alphabetical() {
        let eps = vec![
            ep(1, "Charlie", None),
            ep(2, "alpha", None),
            ep(3, "Bravo", None),
        ];
        let out = filter_sort_paginate(eps, &query(EpisodeOrder::Title, false, 0, 10));
        // Byte-wise: uppercase sorts before lowercase, matching the in-memory path.
        assert_eq!(ids(out), vec![3, 1, 2]);
    }

    #[test]
    fn equal_keys_break_ties_by_id() {
        let eps = vec![
            ep(3, "x", Some(2021)),
            ep(1, "x", Some(2021)),
            ep(2, "x", Some(2021)),
        ];
        let asc =
            filter_sort_paginate(eps.clone(), &query(EpisodeOrder::PublishedAt, false, 0, 10));
        assert_eq!(ids(asc), vec![1, 2, 3], "ascending id tiebreak");
        let desc = filter_sort_paginate(eps, &query(EpisodeOrder::PublishedAt, true, 0, 10));
        assert_eq!(
            ids(desc),
            vec![3, 2, 1],
            "descending reverses the tiebreak too"
        );
    }

    #[test]
    fn paginates_by_page_and_size() {
        let eps: Vec<_> = (0..25)
            .map(|i| ep(i, &format!("{i:04}"), Some(2000 + i)))
            .collect();
        // size 10, page 0/1/2 → 10, 10, 5; pages are disjoint and cover everything.
        let q = |page| query(EpisodeOrder::PublishedAt, true, page, 10);
        assert_eq!(filter_sort_paginate(eps.clone(), &q(0)).len(), 10);
        assert_eq!(filter_sort_paginate(eps.clone(), &q(1)).len(), 10);
        let last = filter_sort_paginate(eps.clone(), &q(2));
        assert_eq!(last.len(), 5);
        // Page 0 newest (2024 → id 24); page 2 oldest tail ends at id 0.
        assert_eq!(filter_sort_paginate(eps.clone(), &q(0))[0].id, 24);
        assert_eq!(last.last().unwrap().id, 0);
        // Past the end → empty (signals no more pages).
        assert!(filter_sort_paginate(eps, &q(3)).is_empty());
    }

    #[test]
    fn filter_narrows_search_status_and_podcast() {
        let mut a = ep(1, "Rust News", Some(2021)); // title match
        a.podcast_id = 7;
        a.playback_status = PlaybackStatus::Finished;
        let mut b = ep(2, "Cooking", Some(2022)); // no match
        b.podcast_id = 8;
        let mut c = ep(3, "About rust crabs", Some(2023)); // title match, unfinished
        c.podcast_id = 7;
        c.playback_status = PlaybackStatus::Unplayed;
        let eps = vec![a, b, c];

        let with = |filter: EpisodeQueryFilter| EpisodeQuery {
            order_by: EpisodeOrder::PublishedAt,
            descending: true,
            page: 0,
            size: 10,
            filter,
        };

        // Search matches title case-insensitively.
        let q = with(EpisodeQueryFilter {
            search: Some("RUST".into()),
            ..Default::default()
        });
        assert_eq!(ids(filter_sort_paginate(eps.clone(), &q)), vec![3, 1]);
        assert_eq!(filter_count(&eps, &q.filter), 2);

        // Playback-status facet matches the row's status exactly (Finished here).
        let q = with(EpisodeQueryFilter {
            playback_status: vec![PlaybackStatus::Finished],
            ..Default::default()
        });
        assert_eq!(ids(filter_sort_paginate(eps.clone(), &q)), vec![1]);

        // Podcast + id-set intersection.
        let q = with(EpisodeQueryFilter {
            podcast_id: Some(7),
            ids: Some(HashSet::from([3])),
            ..Default::default()
        });
        assert_eq!(ids(filter_sort_paginate(eps, &q)), vec![3]);
    }

    /// Regression: `search` matches the SHOW name via `podcast_names` (episode rows
    /// carry no podcast join). Episode 2's own title doesn't contain "radio".
    #[test]
    fn search_matches_podcast_name_via_lookup() {
        let mut a = ep(1, "Cooking", Some(2021));
        a.podcast_id = 7;
        let mut b = ep(2, "Episode 42", Some(2022));
        b.podcast_id = 9;
        let eps = vec![a, b];

        let filter = EpisodeQueryFilter {
            search: Some("radio".into()),
            podcast_names: HashMap::from([
                (7, "Tech Talk".to_string()),
                (9, "Radiolab".to_string()),
            ]),
            ..Default::default()
        };
        let q = EpisodeQuery {
            order_by: EpisodeOrder::PublishedAt,
            descending: true,
            page: 0,
            size: 10,
            filter,
        };
        assert_eq!(ids(filter_sort_paginate(eps.clone(), &q)), vec![2]);
        assert_eq!(filter_count(&eps, &q.filter), 1);

        // Without the lookup populated, the same search finds nothing (the bug).
        let bare = EpisodeQueryFilter {
            search: Some("radio".into()),
            ..Default::default()
        };
        assert_eq!(filter_count(&eps, &bare), 0);
    }
}
