//! Render /latest as a filtered/sorted window over the persistent episode pool. Server pages only add to that pool;
//! filter changes requery cached rows and reset the server cursor without clearing content. Scrolling advances both
//! window and cursor.

use std::collections::HashMap;
use std::rc::Rc;

use dioxus::prelude::*;
use halogen_wire::{
    DefaultListParams, DownloadStatus, EpisodeData, EpisodeInclude, FilterParams, Order,
    OrderDirection as DbOrderDirection, Pagination, PlaybackData, PlaybackStatus,
};

use halogen_apiclient::ApiError;
use halogen_webui_app_state::{PlaybackState, PodcastState};
use halogen_webui_config::api_client_from;
use halogen_webui_listview::{EpisodeFilter, FilterSpec, OrderDirection, SortField, SortSpec};
use halogen_webui_logging::{debug, warn};
use halogen_webui_store::{EpisodeOrder, EpisodeQuery, EpisodeQueryFilter, LocalStore};

/// Episodes fetched per page. Big enough to fill a viewport, small enough that the
/// first page is cheap offline.
pub(super) const PAGE_SIZE: i32 = 30;

/// What [`fetch_server`] needs: where to send the fetched page (the worker) and the session to talk to. The session
/// (`server_url` + `token`) comes from `ClientConfig`, **not** `EpisodeState`, the effect must not subscribe to
/// `EpisodeState`, or the `CacheEpisodes` it dispatches (which the worker publishes back into `EpisodeState`) would
/// retrigger it in a loop.
#[derive(Clone)]
pub(super) struct PagedDeps {
    pub dispatch: Coroutine<halogen_webui_commands::Command>,
    pub server_url: Option<String>,
    pub token: Option<String>,
    /// User-toggled "Go Offline" mode (`ClientConfig.manual_offline`). When set,
    /// the server fetches short-circuit to `Offline` so lists serve the cache only
    /// — matching the worker, which also skips the network while offline.
    pub offline: bool,
}

/// Read the first `visible` filtered + ordered rows from the cache pool, enriched
/// with playback (for progress), plus the total filtered pool count (for
/// `has_more`). This is the render source — re-run whenever the pool changes or
/// the filter/sort/window changes.
pub(super) async fn read_cache(
    store: &Rc<dyn LocalStore>,
    sort: &SortSpec,
    filter: &FilterSpec,
    visible: i32,
    playbacks: &PlaybackState,
    podcasts: &PodcastState,
) -> (Vec<EpisodeData>, usize) {
    let mut query = to_episode_query(sort, filter, visible);
    // Resolve the show name for podcast-name search: episode rows no longer carry
    // the nested podcast join, so the matcher reads titles from the podcast pool.
    // Only needed when searching; skip the clone otherwise.
    if query.filter.search.is_some() {
        query.filter.podcast_names = podcasts
            .podcasts_by_id
            .iter()
            .map(|(id, p)| (*id, p.title.clone()))
            .collect();
    }
    // Tie the page read and the count together: if the page read fails, report an
    // empty pool (count 0) rather than `rows = []` alongside a nonzero count — that
    // split would render "No episodes found" while the scroll gate (`visible <
    // pool_count`) kept firing against an empty render.
    let Ok(page) = store.list_episodes_page(&query).await else {
        return (Vec::new(), 0);
    };
    let rows: Vec<EpisodeData> = page
        .into_iter()
        .map(|ep| enrich_playback(ep, &playbacks.playbacks))
        .collect();
    let count = store.count_episodes(&query.filter).await.unwrap_or(0);
    (rows, count)
}

/// Apply the optimistic-overlay rule to one row's cursor: a locally-written playback (`app_state.playbacks`, e.g. the
/// user seeked) wins; otherwise keep the cursor the server embedded on the body (via `EpisodeInclude::Playback`, cached
/// with it). Never blanks an embedded cursor on an overlay miss, that was the bug when the overlay held the whole set;
/// now it's session-local only.
pub(super) fn enrich_playback(
    mut ep: EpisodeData,
    overlay: &HashMap<i32, PlaybackData>,
) -> EpisodeData {
    if let Some(pb) = overlay.get(&ep.id) {
        ep.playback = Some(pb.clone());
    }
    ep
}

/// Outcome of revalidating one server page against the pool.
pub(super) enum ServerPage {
    /// Fetched + dispatched to the worker; `true` if the server has more pages for
    /// this (sort + filter).
    Fetched { has_more: bool },
    /// No client configured / offline — the cache is all we have.
    Offline,
    /// The server errored.
    Error(String),
}

/// Treat transport unreachability as Offline and keep cached rows, matching manual offline mode. Received HTTP errors
/// or undecodable responses remain errors because the server was reachable and its failure needs feedback.
fn classify_fetch_error(e: ApiError) -> ServerPage {
    if e.is_offline() {
        return ServerPage::Offline;
    }
    ServerPage::Error(e.to_string())
}

/// Fetch one server page for the current (sort + filter) and hand it to the worker
/// to upsert into the pool (single store writer). Does **not** touch what's
/// rendered — `read_cache` re-queries the pool after the upsert lands.
pub(super) async fn fetch_server(
    deps: &PagedDeps,
    sort: &SortSpec,
    filter: &FilterSpec,
    page: i32,
) -> ServerPage {
    if deps.offline {
        return ServerPage::Offline;
    }
    let Some(api) = api_client_from(deps.server_url.as_deref(), deps.token.as_deref()) else {
        return ServerPage::Offline;
    };
    debug!(page, "Fetching episodes page");
    match api.list_episodes(to_list_params(sort, filter, page)).await {
        Ok(result) => {
            let fresh = result.data;
            let has_more = match result.paginator {
                Some(p) => page + 1 < p.pages,
                None => fresh.len() as i32 == PAGE_SIZE,
            };
            debug!(page, count = fresh.len(), has_more, "Fetched episodes page");
            halogen_webui_commands::actions::cache_episodes(&deps.dispatch, fresh);
            ServerPage::Fetched { has_more }
        }
        Err(e) => {
            warn!(page, error = %e, "Failed to fetch episodes page");
            classify_fetch_error(e)
        }
    }
}

// ── Id-list mode: render from an ordered id list, fill missing by id ───────────
// Shared by playlists, device downloads, and playback history — any source whose
// membership is an ordered id list resolved from the pool.

/// Resolve the FULL ordered id list into episode bodies from the pool (no window
/// cap), in list order, skipping ids not yet cached. Used by the Playlist path,
/// which sorts/filters the whole bounded membership before windowing the result.
pub(super) async fn resolve_all(
    store: &Rc<dyn LocalStore>,
    ids: &[i32],
    playbacks: &PlaybackState,
) -> Vec<EpisodeData> {
    let by_id: HashMap<i32, EpisodeData> = store
        .episodes_by_ids(ids)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|e| (e.id, e))
        .collect();
    order_window(ids, &by_id, &playbacks.playbacks)
}

/// Reorder resolved episodes to the window's id order, skipping ids not yet in the
/// pool, and enrich each with its playback (overlay-wins, see [`enrich_playback`]).
/// Pure — the testable core of [`resolve_all`].
fn order_window(
    window: &[i32],
    by_id: &HashMap<i32, EpisodeData>,
    playbacks: &HashMap<i32, PlaybackData>,
) -> Vec<EpisodeData> {
    window
        .iter()
        .filter_map(|id| {
            by_id
                .get(id)
                .cloned()
                .map(|ep| enrich_playback(ep, playbacks))
        })
        .collect()
}

/// The window ids not present in the pool — what [`fetch_by_ids`] must pull.
pub(super) async fn id_window_missing(
    store: &Rc<dyn LocalStore>,
    ids: &[i32],
    visible: i32,
) -> Vec<i32> {
    let window: Vec<i32> = ids.iter().copied().take(visible.max(0) as usize).collect();
    let have: HashMap<i32, ()> = store
        .episodes_by_ids(&window)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|e| (e.id, ()))
        .collect();
    window
        .into_iter()
        .filter(|id| !have.contains_key(id))
        .collect()
}

/// ALL ids not present in the pool — what the Playlist path must pull so a
/// non-Custom sort can order across the whole membership, not just the first
/// window. Bounded by the playlist size; loop-safe (empties once all pooled).
pub(super) async fn id_all_missing(store: &Rc<dyn LocalStore>, ids: &[i32]) -> Vec<i32> {
    let have: HashMap<i32, ()> = store
        .episodes_by_ids(ids)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|e| (e.id, ()))
        .collect();
    ids.iter()
        .copied()
        .filter(|id| !have.contains_key(id))
        .collect()
}

/// Saturating fetch of specific episode ids → hand to the worker to upsert into
/// the pool. The pool is then re-resolved by [`resolve_all`].
pub(super) async fn fetch_by_ids(deps: &PagedDeps, ids: Vec<i32>) -> ServerPage {
    if ids.is_empty() {
        return ServerPage::Fetched { has_more: false };
    }
    if deps.offline {
        return ServerPage::Offline;
    }
    let Some(api) = api_client_from(deps.server_url.as_deref(), deps.token.as_deref()) else {
        return ServerPage::Offline;
    };
    let size = ids.len() as i32;
    let params = DefaultListParams::<EpisodeInclude> {
        pagination: Some(Pagination { page: 0, size }),
        order: None,
        // Embed the caller's resume cursor (progress bars for id-list sources:
        // playlists, downloads, history). NOT the Podcast join — rows resolve the
        // parent name from the pool, avoiding a full podcast copy per episode.
        includes: Some(vec![EpisodeInclude::Playback]),
        filter: Some(FilterParams {
            ids: Some(ids),
            ..Default::default()
        }),
    };
    debug!(count = size, "Fetching episodes by id");
    match api.list_episodes(params).await {
        Ok(result) => {
            debug!(count = result.data.len(), "Fetched episodes by id");
            halogen_webui_commands::actions::cache_episodes(&deps.dispatch, result.data);
            ServerPage::Fetched { has_more: false }
        }
        Err(e) => {
            warn!(error = %e, "Failed to fetch episodes by id");
            classify_fetch_error(e)
        }
    }
}

fn to_list_params(
    sort: &SortSpec,
    filter: &FilterSpec,
    page: i32,
) -> DefaultListParams<EpisodeInclude> {
    DefaultListParams {
        pagination: Some(Pagination {
            page,
            size: PAGE_SIZE,
        }),
        order: Some(Order {
            direction: to_db_direction(sort.direction),
            order_by: order_by_field(sort.field).to_string(),
        }),
        // Embed the caller's resume cursor (cheap, per-row) so progress bars ride
        // the page fetch into the pool. NOT the Podcast join — rows resolve the
        // parent name from the pool (worker `EnsurePodcast` lazy-loads it),
        // avoiding a full podcast copy on every episode.
        includes: Some(vec![EpisodeInclude::Playback]),
        filter: to_filter_params(filter),
    }
}

fn to_episode_query(sort: &SortSpec, filter: &FilterSpec, size: i32) -> EpisodeQuery {
    EpisodeQuery {
        order_by: to_episode_order(sort.field),
        descending: sort.direction == OrderDirection::Desc,
        page: 0,
        size,
        filter: to_query_filter(filter),
    }
}

/// Map the UI `FilterSpec` onto the cache-side `EpisodeQueryFilter` (which can
/// reproduce every facet locally, including the chips).
fn to_query_filter(filter: &FilterSpec) -> EpisodeQueryFilter {
    EpisodeQueryFilter {
        search: filter.search.clone(),
        podcast_id: filter.podcast_id,
        published_after: filter.published_after,
        playback_status: playback_statuses(filter),
        download_status: download_statuses(filter),
        // `/latest` doesn't scope by id; the device-downloads view would set this.
        ids: None,
        // Populated by `read_cache` (which has the EpisodeState pool) only when a search
        // is active — see its body; the bare mapping leaves it empty.
        ..Default::default()
    }
}

/// The playback-status facet, mapped 1:1 onto the 3-state column: Unplayed chip →
/// `Unplayed`, In-Progress (Played) chip → `Played`, Finished chip → `Finished`.
/// Selecting several is an OR (a multi-valued facet the cache narrows; see
/// [`to_filter_params`] for the server's single-value constraint).
fn playback_statuses(filter: &FilterSpec) -> Vec<PlaybackStatus> {
    filter
        .filters
        .iter()
        .filter_map(|f| match f {
            EpisodeFilter::Unplayed => Some(PlaybackStatus::Unplayed),
            EpisodeFilter::Played => Some(PlaybackStatus::Played),
            EpisodeFilter::Finished => Some(PlaybackStatus::Finished),
            _ => None,
        })
        .collect()
}

fn download_statuses(filter: &FilterSpec) -> Vec<DownloadStatus> {
    filter
        .filters
        .iter()
        .filter_map(|f| match f {
            EpisodeFilter::Downloaded => Some(DownloadStatus::Downloaded),
            EpisodeFilter::Downloading => Some(DownloadStatus::Downloading),
            _ => None,
        })
        .collect()
}

/// Map the UI `FilterSpec` onto the server `FilterParams`. The server takes a
/// single status per facet, so a multi-valued facet (e.g. the Unplayed chip, which
/// is two statuses) is omitted here — the server then returns the broader feed and
/// the cache narrows it precisely. Returns `None` when nothing is set.
fn to_filter_params(filter: &FilterSpec) -> Option<FilterParams> {
    let pb = playback_statuses(filter);
    let dl = download_statuses(filter);
    let params = FilterParams {
        search: filter.search.clone(),
        podcast_id: filter.podcast_id,
        download_status: (dl.len() == 1).then(|| dl[0].as_str().to_string()),
        published_after: filter.published_after,
        playback_status: (pb.len() == 1).then(|| pb[0].as_str().to_string()),
        ids: None,
        unplayed_only: None,
    };
    let empty = params.search.is_none()
        && params.podcast_id.is_none()
        && params.download_status.is_none()
        && params.published_after.is_none()
        && params.playback_status.is_none()
        && params.ids.is_none();
    (!empty).then_some(params)
}

fn to_db_direction(dir: OrderDirection) -> DbOrderDirection {
    match dir {
        OrderDirection::Asc => DbOrderDirection::Asc,
        OrderDirection::Desc => DbOrderDirection::Desc,
    }
}

/// Server `order_by` string.
fn order_by_field(field: SortField) -> &'static str {
    match field {
        // Custom (manual position) only exists on the Playlist id-list path, which
        // never reaches the cache-query server sort — fall back to published_at.
        SortField::Custom | SortField::PublishedAt => "published_at",
        SortField::Title => "title",
        SortField::CreatedAt => "created_at",
        SortField::UpdatedAt => "updated_at",
        SortField::Duration => "duration_secs",
    }
}

/// Local-cache ordering, mirroring [`order_by_field`].
fn to_episode_order(field: SortField) -> EpisodeOrder {
    match field {
        SortField::Custom | SortField::PublishedAt => EpisodeOrder::PublishedAt,
        SortField::Title => EpisodeOrder::Title,
        SortField::CreatedAt => EpisodeOrder::CreatedAt,
        SortField::UpdatedAt => EpisodeOrder::UpdatedAt,
        SortField::Duration => EpisodeOrder::Duration,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    fn ep(id: i32) -> EpisodeData {
        let ts = Utc.with_ymd_and_hms(2021, 1, 1, 0, 0, 0).unwrap();
        EpisodeData {
            id,
            podcast_id: 1,
            title: format!("Episode {id}"),
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

    fn pb(episode_id: i32, cursor: u64) -> PlaybackData {
        let ts = Utc.with_ymd_and_hms(2021, 1, 1, 0, 0, 0).unwrap();
        PlaybackData {
            id: 1,
            user_id: 1,
            episode_id,
            cursor,
            completed: false,
            created_at: ts,
            updated_at: ts,
        }
    }

    #[test]
    fn enrich_playback_overlay_wins_and_never_blanks_embedded() {
        // Body carries the cursor the server embedded via `EpisodeInclude::Playback`.
        let mut body = ep(1);
        body.playback = Some(pb(1, 10));

        // Overlay hit (a locally-written cursor) wins.
        let overlay = HashMap::from([(1, pb(1, 99))]);
        assert_eq!(
            enrich_playback(body.clone(), &overlay)
                .playback
                .unwrap()
                .cursor,
            99,
            "optimistic overlay beats the embedded cursor"
        );

        // Overlay miss must PRESERVE the embedded cursor — the bug we're guarding
        // against (blanking it once the overlay is sparse/session-only).
        assert_eq!(
            enrich_playback(body, &HashMap::new())
                .playback
                .unwrap()
                .cursor,
            10,
            "embedded cursor survives an overlay miss"
        );

        // No embedded + no overlay → genuinely None.
        assert!(
            enrich_playback(ep(2), &HashMap::new()).playback.is_none(),
            "no cursor anywhere → None"
        );
    }

    #[test]
    fn order_window_preserves_id_order_skips_missing_and_enriches_playback() {
        let by_id: HashMap<i32, EpisodeData> =
            [ep(3), ep(1)].into_iter().map(|e| (e.id, e)).collect();
        let ts = Utc.with_ymd_and_hms(2021, 1, 1, 0, 0, 0).unwrap();
        let playbacks = HashMap::from([(
            3,
            PlaybackData {
                id: 1,
                user_id: 1,
                episode_id: 3,
                cursor: 9,
                completed: true,
                created_at: ts,
                updated_at: ts,
            },
        )]);

        // Window is the playlist order [3, 1, 2]; 2 isn't in the pool.
        let rows = order_window(&[3, 1, 2], &by_id, &playbacks);

        assert_eq!(
            rows.iter().map(|e| e.id).collect::<Vec<_>>(),
            vec![3, 1],
            "playlist order preserved, missing id (2) skipped"
        );
        assert!(
            rows[0].playback.as_ref().is_some_and(|p| p.completed),
            "row enriched with its playback"
        );
        assert!(rows[1].playback.is_none());
    }

    fn spec(filters: Vec<EpisodeFilter>) -> FilterSpec {
        FilterSpec {
            filters,
            ..Default::default()
        }
    }

    #[test]
    fn default_filter_emits_no_server_params_and_empty_cache_filter() {
        let f = FilterSpec::default();
        assert!(to_filter_params(&f).is_none(), "no filter[...] qs");
        assert!(
            to_query_filter(&f).is_empty(),
            "cache fast path stays available"
        );
    }

    #[test]
    fn search_and_podcast_map_to_both_sides() {
        let f = FilterSpec {
            search: Some("rust".into()),
            podcast_id: Some(7),
            ..Default::default()
        };
        let server = to_filter_params(&f).expect("server filter emitted");
        assert_eq!(server.search.as_deref(), Some("rust"));
        assert_eq!(server.podcast_id, Some(7));
        let cache = to_query_filter(&f);
        assert_eq!(cache.search.as_deref(), Some("rust"));
        assert_eq!(cache.podcast_id, Some(7));
    }

    #[test]
    fn playback_chips_map_one_to_one_on_both_sides() {
        for (chip, status, token) in [
            (
                EpisodeFilter::Unplayed,
                PlaybackStatus::Unplayed,
                "UNPLAYED",
            ),
            (EpisodeFilter::Played, PlaybackStatus::Played, "PLAYED"),
            (
                EpisodeFilter::Finished,
                PlaybackStatus::Finished,
                "FINISHED",
            ),
        ] {
            let f = spec(vec![chip]);
            assert_eq!(to_query_filter(&f).playback_status, vec![status]);
            // Single value → the server can filter it directly.
            assert_eq!(
                to_filter_params(&f).unwrap().playback_status.as_deref(),
                Some(token)
            );
        }
    }

    #[test]
    fn multiple_playback_chips_are_cache_only() {
        // Two statuses in the facet → omitted server-side (the server takes a single
        // value per facet); with nothing else set there's no server filter at all,
        // so the server tops up the broader feed and the cache narrows it precisely.
        let f = spec(vec![EpisodeFilter::Played, EpisodeFilter::Finished]);
        assert_eq!(
            to_query_filter(&f).playback_status,
            vec![PlaybackStatus::Played, PlaybackStatus::Finished]
        );
        assert!(to_filter_params(&f).is_none());
    }

    #[test]
    fn download_chip_maps_to_status() {
        let f = spec(vec![EpisodeFilter::Downloaded]);
        assert_eq!(
            to_query_filter(&f).download_status,
            vec![DownloadStatus::Downloaded]
        );
        assert_eq!(
            to_filter_params(&f).unwrap().download_status.as_deref(),
            Some("DOWNLOADED")
        );
    }

    #[test]
    fn list_params_carry_pagination_order_and_includes() {
        let sort = SortSpec {
            field: SortField::PublishedAt,
            direction: OrderDirection::Desc,
        };
        let params = to_list_params(&sort, &FilterSpec::default(), 2);
        let pag = params.pagination.expect("pagination");
        assert_eq!(pag.page, 2);
        assert_eq!(pag.size, PAGE_SIZE);
        let order = params.order.expect("order");
        assert_eq!(order.order_by, "published_at");
        assert_eq!(order.direction, DbOrderDirection::Desc);
        // Playback embed only — never the heavy Podcast join.
        assert_eq!(params.includes, Some(vec![EpisodeInclude::Playback]));
        assert!(params.filter.is_none());
    }

    #[test]
    fn sort_field_maps_to_server_and_cache_consistently() {
        // Custom (manual position) never reaches the cache-query path, so it
        // degrades to the feed default rather than ordering wrong.
        for (field, col, ord) in [
            (SortField::Custom, "published_at", EpisodeOrder::PublishedAt),
            (
                SortField::PublishedAt,
                "published_at",
                EpisodeOrder::PublishedAt,
            ),
            (SortField::Title, "title", EpisodeOrder::Title),
            (SortField::CreatedAt, "created_at", EpisodeOrder::CreatedAt),
            (SortField::UpdatedAt, "updated_at", EpisodeOrder::UpdatedAt),
            (SortField::Duration, "duration_secs", EpisodeOrder::Duration),
        ] {
            assert_eq!(order_by_field(field), col);
            assert_eq!(to_episode_order(field), ord);
        }
    }

    #[test]
    fn episode_query_is_window_from_page_zero() {
        let q = to_episode_query(
            &SortSpec {
                field: SortField::Title,
                direction: OrderDirection::Asc,
            },
            &FilterSpec::default(),
            45,
        );
        assert_eq!(q.order_by, EpisodeOrder::Title);
        assert!(!q.descending);
        assert_eq!(
            q.page, 0,
            "the cache view is a window from the top, not a page"
        );
        assert_eq!(q.size, 45, "size is the render window");
    }
}
