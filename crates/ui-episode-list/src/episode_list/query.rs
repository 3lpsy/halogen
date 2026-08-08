//! Pure, unit-tested query helpers for the episode list: the id-list builders
//! for id-list sources, the progress fraction, and the in-memory filter + sort.
//!
//! Kept out of `list.rs` so the component there stays focused on rendering and
//! its load effects, while this logic — which has no Dioxus dependency — is
//! testable in isolation.

use halogen_wire::{DownloadStatus, PlaybackStatus, PodcastData};

use halogen_ui_appstate::state::{ClientDownloadState, EpisodeState};
use halogen_ui_appstate::{DownloadState, PlaybackState, PlaylistState};
use halogen_ui_listview::{
    EpisodeData, EpisodeFilter, FilterSpec, ItemVariant, ListSource, OrderDirection, SortField,
    SortSpec,
};

/// The ordered episode-id list backing an **id-list** source, or `None` for a
/// **cache-query** source (paged server-side by filter + sort).
///
/// Id-list sources render a window of these ids resolved from the local pool
/// (missing ids fetched by id), reusing the playlist machinery:
/// - `Playlist` — the pivot membership, in position order.
/// - `ClientDownloads` — device downloads, newest first (id desc ≈ creation order).
/// - `History` — episodes with a playback, most-recently-played first.
pub(super) fn id_list_for_source(
    source: &ListSource,
    app_state: &EpisodeState,
    playlists: &PlaylistState,
    playbacks: &PlaybackState,
    downloads: &DownloadState,
) -> Option<Vec<i32>> {
    match source {
        ListSource::Playlist { id } => Some(
            playlists
                .episodes_by_playlist
                .get(id)
                .cloned()
                .unwrap_or_default(),
        ),
        ListSource::ClientDownloads => {
            // Downloading items appear too (the row's badge shows the spinner);
            // Failed ones don't linger on the Downloads page.
            //
            // Audio blobs are a *shared* device cache (keyed by episode id, not
            // per user), so `client_downloads` can list episodes another account
            // downloaded. Restrict the Downloads page to episodes in THIS user's
            // (namespaced) cache: a shared blob still plays without re-downloading,
            // but you only see downloads for podcasts you actually have.
            let mut ids: Vec<i32> = downloads
                .client_downloads
                .iter()
                .filter(|(_, s)| {
                    matches!(
                        s,
                        ClientDownloadState::Downloading | ClientDownloadState::Downloaded
                    )
                })
                .map(|(id, _)| *id)
                .filter(|id| app_state.episodes_by_id.contains_key(id))
                .collect();
            ids.sort_unstable_by(|a, b| b.cmp(a));
            Some(ids)
        }
        ListSource::ServerDownloads => {
            // Embedded-mode Downloads page: the SERVER's library (its download
            // set) is this device's library. Downloading items appear too, so
            // in-flight fetches are visible; failed states live on the
            // server-errors page instead of lingering here. Newest first.
            let mut ids: Vec<i32> = app_state
                .episodes_by_id
                .values()
                .filter(|e| {
                    matches!(
                        e.download_status,
                        DownloadStatus::Downloading | DownloadStatus::Downloaded
                    )
                })
                .map(|e| e.id)
                .collect();
            ids.sort_unstable_by(|a, b| b.cmp(a));
            Some(ids)
        }
        ListSource::History => {
            let mut rows: Vec<_> = playbacks.playbacks.values().collect();
            // Play-recency desc, tie-broken by episode id desc. The tiebreak is
            // load-bearing: `playbacks` is a `HashMap`, so `.values()` yields an
            // UNSTABLE order that changes as entries are inserted — and a stable sort
            // preserves that order for equal `updated_at`. Without the tiebreak, paging
            // in the next history page (which inserts into the map) reshuffled the
            // ties, swapping already-visible rows mid-scroll (a large CLS). A total
            // order makes the result independent of the map's iteration order.
            rows.sort_by(|a, b| {
                b.updated_at
                    .cmp(&a.updated_at)
                    .then_with(|| b.episode_id.cmp(&a.episode_id))
            });
            Some(rows.into_iter().map(|p| p.episode_id).collect())
        }
        _ => None,
    }
}

/// Downloaded-to-this-device episode ids, newest-first, optionally scoped to one
/// podcast. Backs the `OnDevice` facet on cache-query lists (`/latest`, podcast
/// detail): when that chip is on, the list renders this local set instead of
/// paging the server (which can't express the device-download set). Unlike
/// `ClientDownloads`, this excludes in-flight `Downloading` items.
pub(super) fn id_list_for_device(
    app_state: &EpisodeState,
    downloads: &DownloadState,
    podcast_id: Option<i32>,
) -> Vec<i32> {
    let mut ids: Vec<i32> = downloads
        .client_downloads
        .iter()
        .filter(|(_, s)| **s == ClientDownloadState::Downloaded)
        .map(|(id, _)| *id)
        .filter(|id| match podcast_id {
            Some(pid) => app_state.episodes_by_id.get(id).map(|e| e.podcast_id) == Some(pid),
            // No podcast scope: still restrict to this user's cached episodes, so a
            // blob another account downloaded doesn't leak into the OnDevice facet.
            None => app_state.episodes_by_id.contains_key(id),
        })
        .collect();
    ids.sort_unstable_by(|a, b| b.cmp(a));
    ids
}

/// The multiselect "Selected" view's id list: the selected episode ids, newest
/// first (id desc), optionally scoped to one podcast (podcast-detail list). Backs
/// the `selected_only` chip — when on, the list renders this set instead of the
/// source's normal contents, so a user can review/act on exactly what they ticked.
pub(super) fn id_list_for_selected(
    selected: &std::collections::HashSet<i32>,
    podcast_id: Option<i32>,
    app_state: &EpisodeState,
) -> Vec<i32> {
    let mut ids: Vec<i32> = selected
        .iter()
        .copied()
        .filter(|id| match podcast_id {
            Some(pid) => app_state.episodes_by_id.get(id).map(|e| e.podcast_id) == Some(pid),
            None => true,
        })
        .collect();
    ids.sort_unstable_by(|a, b| b.cmp(a));
    ids
}

/// The id list backing the **fetch** side of the paged list (which ids' bodies to
/// resolve from the pool), or `None` for a cache-query source (server-paged by
/// filter + sort). This is the single source of the id-list **taxonomy**: the
/// `selected` chip wins, then the `id_list_mode` sources (playlist / downloads /
/// history, or an OnDevice cache-query list), else a cache query.
///
/// The render effect mirrors this split inline rather than calling this, because
/// each branch *resolves* differently (windowed vs full, a Custom rank map, the
/// selected chip's bypassed download/played facets) — only the id derivation is
/// shared.
pub(super) fn id_list_for_render(
    source: &ListSource,
    podcast_id: Option<i32>,
    selected_only: bool,
    selected: &std::collections::HashSet<i32>,
    id_list_mode: bool,
    app_state: &EpisodeState,
    playlists: &PlaylistState,
    playbacks: &PlaybackState,
    downloads: &DownloadState,
) -> Option<Vec<i32>> {
    if selected_only {
        return Some(id_list_for_selected(selected, podcast_id, app_state));
    }
    if !id_list_mode {
        return None;
    }
    match source {
        // AllEpisodes is in `id_list_mode` only via the OnDevice facet, so here it
        // means "render the device set" — fetch those ids' bodies.
        ListSource::AllEpisodes => Some(id_list_for_device(app_state, downloads, podcast_id)),
        _ => id_list_for_source(source, app_state, playlists, playbacks, downloads),
    }
}

/// Playback progress fraction (0..1) for the row's progress bar, or `None` when
/// there's nothing meaningful to show.
pub(super) fn compute_progress(episode: &EpisodeData, variant: &ItemVariant) -> Option<f32> {
    if *variant != ItemVariant::WithProgress {
        return None;
    }
    let playback = episode.playback.as_ref()?;
    let duration = episode.duration_secs.filter(|d| *d > 0)?;
    let frac = (playback.cursor as f64 / duration as f64).clamp(0.0, 1.0) as f32;
    // Show a bar for episodes with real progress — started but not at the very
    // end. A finished episode resets its cursor to 0 (so no bar), and an unplayed
    // one has no playback row; re-playing a finished episode shows progress again.
    (frac > 0.0 && frac < 1.0).then_some(frac)
}

/// Pure filter + sort over a set of episodes. Honors `sort.direction` for every
/// field. A standalone helper `load_page` calls, so it can be unit-tested.
pub(super) fn apply_filter_sort(
    episodes: Vec<EpisodeData>,
    sort: &SortSpec,
    filter: &FilterSpec,
    // id -> rank for `SortField::Custom` (the playlist's position order). `None`
    // for non-playlist callers; Custom then degrades to id order (unreachable in
    // practice — Custom is only exposed for `Playlist` sources).
    custom_rank: Option<&std::collections::HashMap<i32, usize>>,
    // The EpisodeState podcast pool, so `search` can match the show name: episode rows
    // no longer carry the nested podcast join, so the parent name is resolved here
    // by `podcast_id` (mirrors the cache-query path's `EpisodeQueryFilter`).
    podcasts: &std::collections::HashMap<i32, PodcastData>,
) -> Vec<EpisodeData> {
    let mut out: Vec<EpisodeData> = episodes
        .into_iter()
        .filter(|ep| {
            if let Some(search) = &filter.search {
                let q = search.to_lowercase();
                let in_title = ep.title.to_lowercase().contains(&q);
                let in_summary = ep
                    .description
                    .as_ref()
                    .is_some_and(|s| s.to_lowercase().contains(&q));
                // Match the podcast name too, so search behaves the same here as on
                // the cache-query path (`EpisodeQueryFilter::matches`). Resolved from
                // the pool by `podcast_id`, not a nested join the row no longer holds.
                let in_podcast = podcasts
                    .get(&ep.podcast_id)
                    .is_some_and(|p| p.title.to_lowercase().contains(&q));
                if !in_title && !in_summary && !in_podcast {
                    return false;
                }
            }
            if let Some(podcast_id) = filter.podcast_id
                && ep.podcast_id != podcast_id
            {
                return false;
            }
            if let Some(after) = filter.published_after
                && let Some(pub_at) = ep.published_at
                && pub_at < after
            {
                return false;
            }
            // Multi-select filter chips: OR within a facet, AND across facets.
            // Download facet (Downloaded / Downloading).
            let dl_selected: Vec<EpisodeFilter> = filter
                .filters
                .iter()
                .copied()
                .filter(|f| matches!(f, EpisodeFilter::Downloaded | EpisodeFilter::Downloading))
                .collect();
            if !dl_selected.is_empty() {
                let ok = dl_selected.iter().any(|f| match f {
                    EpisodeFilter::Downloaded => ep.download_status == DownloadStatus::Downloaded,
                    EpisodeFilter::Downloading => ep.download_status == DownloadStatus::Downloading,
                    _ => false,
                });
                if !ok {
                    return false;
                }
            }
            // Played-state facet (Unplayed / Played / Finished) — matched 1:1
            // against the row's per-user `playback_status` (the same 3-state column
            // the cache-query path filters on, so id-list lists agree with /latest).
            let played_selected: Vec<EpisodeFilter> = filter
                .filters
                .iter()
                .copied()
                .filter(|f| {
                    matches!(
                        f,
                        EpisodeFilter::Unplayed | EpisodeFilter::Played | EpisodeFilter::Finished
                    )
                })
                .collect();
            if !played_selected.is_empty() {
                let status = &ep.playback_status;
                let ok = played_selected.iter().any(|f| match f {
                    EpisodeFilter::Unplayed => *status == PlaybackStatus::Unplayed,
                    EpisodeFilter::Played => *status == PlaybackStatus::Played,
                    EpisodeFilter::Finished => *status == PlaybackStatus::Finished,
                    _ => false,
                });
                if !ok {
                    return false;
                }
            }
            true
        })
        .collect();

    out.sort_by(|a, b| {
        let cmp = match sort.field {
            SortField::Custom => {
                let ra = custom_rank
                    .and_then(|m| m.get(&a.id))
                    .copied()
                    .unwrap_or(usize::MAX);
                let rb = custom_rank
                    .and_then(|m| m.get(&b.id))
                    .copied()
                    .unwrap_or(usize::MAX);
                ra.cmp(&rb).then_with(|| a.id.cmp(&b.id))
            }
            SortField::PublishedAt => a
                .published_at
                .cmp(&b.published_at)
                .then_with(|| a.id.cmp(&b.id)),
            SortField::Title => a.title.cmp(&b.title).then_with(|| a.id.cmp(&b.id)),
            SortField::CreatedAt => a
                .created_at
                .cmp(&b.created_at)
                .then_with(|| a.id.cmp(&b.id)),
            SortField::UpdatedAt => a
                .updated_at
                .cmp(&b.updated_at)
                .then_with(|| a.id.cmp(&b.id)),
            SortField::Duration => a
                .duration_secs
                .cmp(&b.duration_secs)
                .then_with(|| a.id.cmp(&b.id)),
        };
        match sort.direction {
            OrderDirection::Asc => cmp,
            OrderDirection::Desc => cmp.reverse(),
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use super::*;
    use chrono::{TimeZone, Utc};
    use halogen_wire::{DownloadStatus, PlaybackStatus, PodcastData};

    /// No podcasts → podcast-name search is off; the call sites that don't exercise
    /// show-name matching pass this.
    fn no_podcasts() -> HashMap<i32, PodcastData> {
        HashMap::new()
    }

    fn pod(id: i32, title: &str) -> PodcastData {
        let ts = Utc.with_ymd_and_hms(2021, 1, 1, 0, 0, 0).unwrap();
        PodcastData {
            id,
            title: title.to_string(),
            description: String::new(),
            feed_url: String::new(),
            art_url: None,
            author: None,
            polled_at: None,
            podcast_config_id: None,
            art_file_path: None,
            etag: None,
            last_modified: None,
            podcast_config: None,
            created_at: ts,
            updated_at: ts,
            episode_count: None,
            feed_url_redirects: None,
        }
    }

    fn ep(id: i32, title: &str, podcast_id: i32, year: i32) -> EpisodeData {
        let ts = Utc.with_ymd_and_hms(year, 1, 1, 0, 0, 0).unwrap();
        EpisodeData {
            id,
            podcast_id,
            title: title.to_string(),
            description: None,
            content_url: String::new(),
            guid: None,
            art_url: None,
            published_at: Some(ts),
            downloaded_at: None,
            content_file_path: None,
            download_size: None,
            art_file_path: None,
            download_status: DownloadStatus::NotDownloaded,
            download_started_at: None,
            download_attempts: 0,
            playback_status: PlaybackStatus::Unplayed,
            duration_secs: None,
            created_at: ts,
            updated_at: ts,
            podcast: None,
            playback: None,
            chapters: None,
        }
    }

    #[test]
    fn id_list_for_source_orders_downloads_and_history() {
        use halogen_ui_appstate::state::ClientDownloadState;
        use halogen_wire::PlaybackData;

        let mut s = EpisodeState::default();
        let mut d = DownloadState::default();

        // Device downloads render newest-first (id desc ≈ creation order);
        // in-flight items show up, failed ones don't.
        d.client_downloads = HashMap::from([
            (3, ClientDownloadState::Downloaded),
            (1, ClientDownloadState::Downloading),
            (2, ClientDownloadState::Downloaded),
            (9, ClientDownloadState::Failed),
        ]);
        // The Downloads page lists only episodes in THIS user's cache (audio blobs
        // are a shared device cache, so `client_downloads` can name another
        // account's downloads). Seed the bodies for the qualifying ids.
        s.episodes_by_id = HashMap::from([
            (1, ep(1, "a", 7, 2021)),
            (2, ep(2, "b", 7, 2021)),
            (3, ep(3, "c", 7, 2021)),
        ]);
        let p = PlaylistState::default();
        let mut pb_state = PlaybackState::default();
        assert_eq!(
            id_list_for_source(&ListSource::ClientDownloads, &s, &p, &pb_state, &d),
            Some(vec![3, 2, 1])
        );

        // History: episodes ordered by playback recency (updated_at desc).
        let pb = |episode_id: i32, minute: u32| PlaybackData {
            id: episode_id,
            user_id: 1,
            episode_id,
            cursor: 0,
            completed: false,
            created_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            updated_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, minute, 0).unwrap(),
        };
        let mut pbs = HashMap::new();
        pbs.insert(10, pb(10, 5)); // most recent
        pbs.insert(20, pb(20, 1)); // oldest
        pbs.insert(30, pb(30, 3));
        pb_state.playbacks = pbs;
        assert_eq!(
            id_list_for_source(&ListSource::History, &s, &p, &pb_state, &d),
            Some(vec![10, 30, 20])
        );

        // Ties in `updated_at` must resolve deterministically (episode id desc), not
        // by the HashMap's iteration order — otherwise paging (which inserts into the
        // map) reshuffles ties and swaps visible rows mid-scroll. Same minute for all,
        // inserted out of order, across enough entries to exercise the map.
        let mut tied = HashMap::new();
        for id in [40, 10, 25, 5, 33, 18, 60, 2] {
            tied.insert(id, pb(id, 7));
        }
        pb_state.playbacks = tied;
        assert_eq!(
            id_list_for_source(&ListSource::History, &s, &p, &pb_state, &d),
            Some(vec![60, 40, 33, 25, 18, 10, 5, 2]),
            "tied updated_at must fall back to episode id desc, independent of map order"
        );

        // A cache-query source has no id list (it pages the server by filter+sort).
        assert_eq!(
            id_list_for_source(&ListSource::AllEpisodes, &s, &p, &pb_state, &d),
            None
        );
    }

    #[test]
    fn id_list_for_render_follows_the_taxonomy() {
        let mut s = EpisodeState::default();
        s.episodes_by_id = HashMap::from([(1, ep(1, "a", 7, 2021))]);
        let p = PlaylistState::default();
        let pb = PlaybackState::default();
        let d = DownloadState::default();
        let empty = HashSet::new();

        // Cache-query source, not id-list mode → None (pages the server).
        assert_eq!(
            id_list_for_render(
                &ListSource::AllEpisodes,
                None,
                false,
                &empty,
                false,
                &s,
                &p,
                &pb,
                &d
            ),
            None
        );
        // The "Selected" chip wins regardless of source / mode.
        let sel: HashSet<i32> = [1].into_iter().collect();
        assert_eq!(
            id_list_for_render(
                &ListSource::AllEpisodes,
                None,
                true,
                &sel,
                false,
                &s,
                &p,
                &pb,
                &d
            ),
            Some(vec![1])
        );
        // id-list mode + AllEpisodes → the device set (empty here).
        assert_eq!(
            id_list_for_render(
                &ListSource::AllEpisodes,
                None,
                false,
                &empty,
                true,
                &s,
                &p,
                &pb,
                &d
            ),
            Some(vec![])
        );
        // id-list mode + Playlist → its membership (empty playlist → empty list).
        assert_eq!(
            id_list_for_render(
                &ListSource::Playlist { id: 9 },
                None,
                false,
                &empty,
                true,
                &s,
                &p,
                &pb,
                &d
            ),
            Some(vec![])
        );
    }

    #[test]
    fn id_list_for_selected_scopes_and_orders() {
        let mut s = EpisodeState::default();
        s.episodes_by_id = HashMap::from([
            (1, ep(1, "a", 7, 2021)),
            (2, ep(2, "b", 7, 2021)),
            (3, ep(3, "c", 9, 2021)),
        ]);
        let selected: HashSet<i32> = [1, 2, 3].into_iter().collect();

        // Unscoped: every selected id, newest-first (id desc).
        assert_eq!(id_list_for_selected(&selected, None, &s), vec![3, 2, 1]);
        // Scoped to podcast 7: only its episodes (3 belongs to podcast 9).
        assert_eq!(id_list_for_selected(&selected, Some(7), &s), vec![2, 1]);

        // An id with no body in the pool is kept when unscoped (the selection is the
        // source of truth), but dropped when scoped — its podcast can't be confirmed.
        let mixed: HashSet<i32> = [3, 99].into_iter().collect();
        assert_eq!(id_list_for_selected(&mixed, None, &s), vec![99, 3]);
        assert_eq!(id_list_for_selected(&mixed, Some(9), &s), vec![3]);
    }

    #[test]
    fn id_list_for_device_downloaded_only_newest_first_and_podcast_scoped() {
        use halogen_ui_appstate::state::ClientDownloadState;

        let mut s = EpisodeState::default();
        let mut d = DownloadState::default();
        // Only fully-Downloaded items qualify; Downloading/Failed are excluded.
        d.client_downloads = HashMap::from([
            (1, ClientDownloadState::Downloaded),
            (2, ClientDownloadState::Downloading),
            (3, ClientDownloadState::Downloaded),
            (4, ClientDownloadState::Failed),
        ]);
        // Bodies needed only for podcast scoping: 1 → podcast 7, 3 → podcast 9.
        s.episodes_by_id = HashMap::from([(1, ep(1, "a", 7, 2021)), (3, ep(3, "c", 9, 2021))]);

        // Unscoped: downloaded-only, newest-first (id desc).
        assert_eq!(id_list_for_device(&s, &d, None), vec![3, 1]);
        // Scoped to podcast 7: only episode 1 (3 belongs to podcast 9).
        assert_eq!(id_list_for_device(&s, &d, Some(7)), vec![1]);
    }

    #[test]
    fn sort_direction_is_honored() {
        let eps = vec![
            ep(1, "a", 1, 2020),
            ep(2, "b", 1, 2022),
            ep(3, "c", 1, 2021),
        ];
        let asc = apply_filter_sort(
            eps.clone(),
            &SortSpec {
                field: SortField::PublishedAt,
                direction: OrderDirection::Asc,
            },
            &FilterSpec::default(),
            None,
            &no_podcasts(),
        );
        assert_eq!(asc.iter().map(|e| e.id).collect::<Vec<_>>(), vec![1, 3, 2]);
        let desc = apply_filter_sort(
            eps,
            &SortSpec {
                field: SortField::PublishedAt,
                direction: OrderDirection::Desc,
            },
            &FilterSpec::default(),
            None,
            &no_podcasts(),
        );
        assert_eq!(desc.iter().map(|e| e.id).collect::<Vec<_>>(), vec![2, 3, 1]);
    }

    #[test]
    fn custom_sort_uses_rank_and_still_filters() {
        let eps = vec![
            ep(1, "a", 1, 2020),
            ep(2, "b", 1, 2022),
            ep(3, "c", 1, 2021),
        ];
        // Rank puts 3 first, then 1, then 2 (the playlist's position order).
        let rank: HashMap<i32, usize> = [(3, 0), (1, 1), (2, 2)].into_iter().collect();
        let ordered = apply_filter_sort(
            eps.clone(),
            &SortSpec {
                field: SortField::Custom,
                direction: OrderDirection::Asc,
            },
            &FilterSpec::default(),
            Some(&rank),
            &no_podcasts(),
        );
        assert_eq!(
            ordered.iter().map(|e| e.id).collect::<Vec<_>>(),
            vec![3, 1, 2]
        );

        // Filtering still applies under Custom.
        let filtered = apply_filter_sort(
            eps,
            &SortSpec {
                field: SortField::Custom,
                direction: OrderDirection::Asc,
            },
            &FilterSpec {
                search: Some("c".into()),
                ..Default::default()
            },
            Some(&rank),
            &no_podcasts(),
        );
        assert_eq!(filtered.iter().map(|e| e.id).collect::<Vec<_>>(), vec![3]);
    }

    /// Build a playback row with the given cursor for `compute_progress` tests.
    fn playback(episode_id: i32, cursor: u64) -> halogen_wire::PlaybackData {
        halogen_wire::PlaybackData {
            id: episode_id,
            user_id: 1,
            episode_id,
            cursor,
            completed: false,
            created_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            updated_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
        }
    }

    #[test]
    fn compute_progress_none_for_non_progress_variant() {
        let mut e = ep(1, "a", 1, 2021);
        e.duration_secs = Some(100);
        e.playback = Some(playback(1, 50));
        assert_eq!(compute_progress(&e, &ItemVariant::Default), None);
    }

    #[test]
    fn compute_progress_none_without_playback_or_duration() {
        // No playback row.
        let mut e = ep(1, "a", 1, 2021);
        e.duration_secs = Some(100);
        assert_eq!(compute_progress(&e, &ItemVariant::WithProgress), None);

        // Playback present but no duration.
        let mut e = ep(1, "a", 1, 2021);
        e.playback = Some(playback(1, 50));
        e.duration_secs = None;
        assert_eq!(compute_progress(&e, &ItemVariant::WithProgress), None);

        // Duration zero is treated as unknown.
        let mut e = ep(1, "a", 1, 2021);
        e.playback = Some(playback(1, 50));
        e.duration_secs = Some(0);
        assert_eq!(compute_progress(&e, &ItemVariant::WithProgress), None);
    }

    #[test]
    fn compute_progress_none_at_zero_and_full() {
        // frac == 0.0 (cursor 0): no bar.
        let mut e = ep(1, "a", 1, 2021);
        e.duration_secs = Some(100);
        e.playback = Some(playback(1, 0));
        assert_eq!(compute_progress(&e, &ItemVariant::WithProgress), None);

        // frac >= 1.0 (cursor == duration, clamped to 1.0): no bar.
        let mut e = ep(1, "a", 1, 2021);
        e.duration_secs = Some(100);
        e.playback = Some(playback(1, 100));
        assert_eq!(compute_progress(&e, &ItemVariant::WithProgress), None);

        // Cursor beyond duration also clamps to 1.0 → no bar.
        let mut e = ep(1, "a", 1, 2021);
        e.duration_secs = Some(100);
        e.playback = Some(playback(1, 250));
        assert_eq!(compute_progress(&e, &ItemVariant::WithProgress), None);
    }

    #[test]
    fn compute_progress_some_mid_progress() {
        let mut e = ep(1, "a", 1, 2021);
        e.duration_secs = Some(200);
        e.playback = Some(playback(1, 50));
        let frac = compute_progress(&e, &ItemVariant::WithProgress).unwrap();
        assert!((frac - 0.25).abs() < 1e-6, "got {frac}");
    }

    #[test]
    fn filters_search_and_podcast() {
        let eps = vec![ep(1, "Rust News", 1, 2021), ep(2, "Cooking", 2, 2021)];
        let by_search = apply_filter_sort(
            eps.clone(),
            &SortSpec::default(),
            &FilterSpec {
                search: Some("rust".into()),
                ..Default::default()
            },
            None,
            &no_podcasts(),
        );
        assert_eq!(by_search.len(), 1);
        assert_eq!(by_search[0].id, 1);

        let by_podcast = apply_filter_sort(
            eps,
            &SortSpec::default(),
            &FilterSpec {
                podcast_id: Some(2),
                ..Default::default()
            },
            None,
            &no_podcasts(),
        );
        assert_eq!(by_podcast.len(), 1);
        assert_eq!(by_podcast[0].id, 2);
    }

    /// Regression: searching by SHOW name matches episodes whose podcast title
    /// contains the term, resolved from the pool (episode rows carry no podcast
    /// join). Episode 2's own title/description don't contain "radio".
    #[test]
    fn search_matches_podcast_name_from_pool() {
        let eps = vec![ep(1, "Cooking", 1, 2021), ep(2, "Episode 42", 2, 2021)];
        let podcasts = HashMap::from([(1, pod(1, "Tech Talk")), (2, pod(2, "Radiolab"))]);

        let hits = apply_filter_sort(
            eps,
            &SortSpec::default(),
            &FilterSpec {
                search: Some("radio".into()),
                ..Default::default()
            },
            None,
            &podcasts,
        );
        assert_eq!(hits.len(), 1, "episode whose show name matches is found");
        assert_eq!(hits[0].id, 2);
    }

    #[test]
    fn played_facet_partitions_by_three_state_status() {
        let mut unplayed = ep(1, "a", 1, 2021);
        unplayed.playback_status = PlaybackStatus::Unplayed;
        let mut in_progress = ep(2, "b", 1, 2021);
        in_progress.playback_status = PlaybackStatus::Played;
        let mut finished = ep(3, "c", 1, 2021);
        finished.playback_status = PlaybackStatus::Finished;
        let eps = vec![unplayed, in_progress, finished];

        let only = |chip: EpisodeFilter| {
            apply_filter_sort(
                eps.clone(),
                &SortSpec::default(),
                &FilterSpec {
                    filters: vec![chip],
                    ..Default::default()
                },
                None,
                &no_podcasts(),
            )
            .into_iter()
            .map(|e| e.id)
            .collect::<Vec<_>>()
        };

        // Each chip selects exactly its status — no overlap.
        assert_eq!(only(EpisodeFilter::Unplayed), vec![1]);
        assert_eq!(only(EpisodeFilter::Played), vec![2]);
        assert_eq!(only(EpisodeFilter::Finished), vec![3]);

        // Two chips OR within the facet (in-progress OR finished).
        let mut both = apply_filter_sort(
            eps,
            &SortSpec {
                field: SortField::PublishedAt,
                direction: OrderDirection::Asc,
            },
            &FilterSpec {
                filters: vec![EpisodeFilter::Played, EpisodeFilter::Finished],
                ..Default::default()
            },
            None,
            &no_podcasts(),
        )
        .into_iter()
        .map(|e| e.id)
        .collect::<Vec<_>>();
        both.sort_unstable();
        assert_eq!(both, vec![2, 3]);
    }
}
