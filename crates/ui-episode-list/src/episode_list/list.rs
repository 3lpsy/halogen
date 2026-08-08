use dioxus::prelude::*;

use halogen_ui_logging::debug;

use super::bulk_menu::bulk_menu_sections;
use super::paged;
use super::query::{
    apply_filter_sort, compute_progress, id_list_for_device, id_list_for_render,
    id_list_for_selected, id_list_for_source,
};
use super::{EpisodeListItem, ListControls};
use halogen_ui_appstate::state::{ClientDownloadState, EpisodeState};
use halogen_ui_appstate::{DownloadState, PlaybackState, PlaylistState, PodcastState};
use halogen_ui_listview::{
    EpisodeData, EpisodeFilter, FilterSpec, ItemVariant, ListSource, MultiSelectState,
    OrderDirection, SortField, SortSpec,
};
use halogen_ui_state::hooks::{
    FetchResult, PagedPool, Revalidate, ScrollPaging, infinite_scroll_body, list_stamp, use_config,
    use_dispatch, use_dom_stream, use_downloads, use_history, use_is_admin, use_is_offline,
    use_latest_wins, use_paged_pool_with, use_playbacks, use_playlists, use_podcasts,
    use_pull_to_refresh, use_row_memory, use_scroll_memory, use_store, use_toast,
};
use halogen_ui_widgets::{PullToRefreshIndicator, use_quick_menu};

#[component]
pub fn EpisodeList(
    source: ListSource,
    sort: Signal<SortSpec>,
    filter: Signal<FilterSpec>,
    swipe: halogen_ui_listview::SwipeConfig,
    item_variant: ItemVariant,
    app_state: ReadSignal<EpisodeState>,
    /// When set, page episodes from the server (offline-first, stale-while-
    /// revalidate) instead of rendering the whole in-memory set. Default `false`
    /// keeps every existing consumer on the in-memory path.
    #[props(default)]
    paged: bool,
    /// Opt-in in-memory scroll-position memory, keyed by list type (e.g. "latest",
    /// "queue", "downloads"). `None` (the default) disables it — used by consumers
    /// like podcast/playlist detail that shouldn't remember scroll.
    #[props(default)]
    scroll_key: Option<&'static str>,
    /// Opt-in rows memory WITHOUT scroll memory (detail pages): the row stamp is
    /// source-scoped so one key per page type can't leak rows across ids.
    #[props(default)]
    rows_key: Option<&'static str>,
) -> Element {
    let mut episodes = use_signal::<Vec<EpisodeData>>(Vec::new);
    // `pool_count` is the filtered pool size (drives the reorder mirror axis and the
    // History page-ahead gate); `cache_version` bumps when a fetch upserts the pool,
    // re-querying the render (effect A). The cursor/window/loading/error/refresh
    // signals + the infinite-scroll observer all live in `use_paged_pool` below.
    let mut pool_count = use_signal(|| 0usize);
    // The id-list total for id-list sources (History/Downloads/Playlist/Selected):
    // the UNRESOLVED membership size, before body resolution and filtering.
    // `pool_count` alone cannot gate window growth there — it counts only rows
    // whose bodies are already in the local store, and bodies are only fetched up
    // to `visible`, so `visible < pool_count` deadlocks the moment the cache
    // lacks bodies past the window (growth needs resolved rows, resolving needs
    // growth). Growth is gated on `max(pool_count, id_total)` instead: the
    // window may advance past the resolved set, which re-runs the window-body
    // fetch for the newly-revealed ids. Bounded by the membership size.
    let mut id_total = use_signal(|| 0usize);
    let mut cache_version = use_signal(|| 0u32);

    // History (server-paged `/playbacks` source) paging guards: `inflight` dedups
    // overlapping page requests; `seen_page` detects a landed page (the worker
    // advanced `history_next_page`) to clear `inflight`. Unused for other sources.
    let mut history_inflight = use_signal(|| false);
    // Dedup guard for the id-list body-fetch effect below. Without it, the effect
    // (which subscribes to EpisodeState/playlists/playbacks/downloads) re-fires on
    // every worker publish and re-issues the SAME `filter[ids]` fetch before the
    // first one's bodies land in the store — triple-fetching the playlist at boot.
    // `peek`ed, never subscribed: convergence rides the effect's real-state
    // subscriptions (a landed fetch republishes EpisodeState → effect re-runs with
    // the guard already cleared), so it can't self-loop.
    let mut idlist_inflight = use_signal(|| false);
    let mut history_seen_page = use_signal(|| 0i32);

    // Multiselect (bulk-action) view state — page-local, never persisted. Drives
    // the checkboxes, the count badge, the "Selected" review chip, and gating of
    // swipe / pull-to-refresh / reorder while active.
    let mut ms = use_signal(MultiSelectState::default);

    // Paged-mode dependencies (cheap; unused on the in-memory path).
    let store = use_store();
    let dispatch = use_dispatch();
    let config = use_config();
    let quick = use_quick_menu();
    let toast = use_toast();
    let nav = use_navigator();

    // `StoreHandle`/`ListSource` aren't `Copy`; clone one per closure that needs them.
    let store_a = store.0.clone();
    let store_b = store.0.clone();
    let source_for_render = source.clone();
    let source_for_idlist = source.clone();
    let source_for_mode = source.clone();
    let source_for_inmem = source.clone();
    let source_for_history = source.clone();
    let source_for_rowmem = source.clone();
    // History uniquely sources its membership from server-paged `/playbacks`
    // (merged into the `playbacks` overlay by the worker), not from a fixed
    // in-memory id list — so it gets its own load + scroll-paging effects below.
    let is_history = matches!(source, ListSource::History);
    let app_state_clone = app_state;
    // Device/server download state — its own signal now. Cloned per closure that
    // resolves id-lists / device facets (Signal is `Copy`).
    let downloads = use_downloads();
    let downloads_clone = downloads;
    // Podcast pool — its own signal now. Snapshotted alongside `app_state` in the
    // effects that resolve podcast names (search-by-show, render enrichment), so a
    // podcast title landing (lazy `EnsurePodcast`) re-runs them, exactly as it did
    // when podcasts lived in `EpisodeState`.
    let podcasts = use_podcasts();
    let podcasts_clone = podcasts;
    // Playlists + queue — its own signal now. Snapshotted alongside `app_state` in
    // the id-list / render effects so a playlist membership or queue change lands the
    // same way it did when it lived in `EpisodeState`.
    let playlists = use_playlists();
    let playlists_clone = playlists;
    // Playback overlay — its own signal now. Snapshotted in the render/id-list
    // effects so a cursor save (progress) and the History membership land the same
    // way they did when playbacks lived in `EpisodeState`.
    let playbacks = use_playbacks();
    let playbacks_clone = playbacks;
    // History `/playbacks` paging cursor — its own signal now. The page-ahead effect
    // subscribes to it so advancing the cursor (a page landed) re-runs the effect.
    let history = use_history();
    // Reactive: the OnDevice chip turns a cache-query list (/latest, podcast
    // detail) into a device id-list — small + fully local, so no sparse server
    // pages. Recomputed when the filter changes, so toggling OnDevice flips modes.
    let id_list_mode = use_memo(move || {
        // The "Selected" chip renders a bounded local set, so it's an id-list for
        // every source (no server paging of a client-only selection).
        ms.read().selected_only
            || source_for_mode.is_id_list()
            || (matches!(source_for_mode, ListSource::AllEpisodes)
                && filter.read().filters.contains(&EpisodeFilter::OnDevice))
    });

    // The multiselect GATE (`selected_only`) read at PartialEq granularity. Reading
    // `ms.read().selected_only` directly inside the pool/render effects subscribes
    // them to the WHOLE `ms` signal (Dioxus subscribes per-signal, not per-field),
    // so every checkbox tick would re-fire the async store re-query + full re-render.
    // This memo only notifies when the gate actually TOGGLES; the selection SET is
    // then `.read()` (subscribed) ONLY while the gate is on, `.peek()`-discipline off.
    let selected_only = use_memo(move || ms.read().selected_only);

    // Offline-first paged engine (shared with the Podcasts/Playlists pages): owns
    // the cursor/window/loading/error/refresh signals. The `plan` here handles the
    // CACHE-QUERY sources (/latest-style: server page → pool). In-memory mode
    // (`paged == false`), offline, and the id-list sources all `Skip`: rendering
    // then comes from the in-memory effect / cached pool, and the id-list bodies are
    // fetched by the dedicated effect below.
    //
    // Scroll is managed by this component (`use_paged_pool_with(None, …)`), not the
    // hook: the episode list is multi-mode (cache-query / id-list / in-memory) and
    // must gate window growth on `pool_count` so parking at the bottom of a fully-
    // loaded list doesn't re-query the store every tick — see the dom-stream below.
    //
    // The cache-query plan must NOT subscribe to EpisodeState (it reads only
    // config/sort/filter): its own fetched page is upserted + published by the
    // worker, and a subscription would retrigger it in a tight loop. On a successful
    // fetch it bumps `cache_version` so effect A re-queries the pool.
    // `None` scroll: this list manages its own infinite-scroll (the `use_dom_stream`
    // below), so the built-in handler never fires and `cached_len` is unused here.
    let pool = use_paged_pool_with(
        None,
        || 0,
        move |server_page| {
            // In-memory mode + id-list sources don't page-fetch through the pool.
            // `id_list_mode()` already covers `selected_only` (the Selected chip is an
            // id-list for every source), so don't also read `ms` here — that would
            // re-subscribe this plan to every checkbox tick.
            if !paged || id_list_mode() {
                return Revalidate::Skip;
            }
            let cfg = config.read();
            if cfg.manual_offline {
                return Revalidate::Skip; // "Go Offline": serve the cached pool only
            }
            let deps = paged::PagedDeps {
                dispatch,
                server_url: cfg.server_url.clone(),
                token: cfg.access_token.clone(),
                offline: cfg.manual_offline,
            };
            drop(cfg);
            let sort_val = sort.read().clone();
            let filter_val = filter.read().clone();
            Revalidate::Fetch(Box::pin(async move {
                match paged::fetch_server(&deps, &sort_val, &filter_val, server_page).await {
                    paged::ServerPage::Fetched { has_more } => {
                        cache_version += 1;
                        FetchResult {
                            has_more,
                            error: None,
                        }
                    }
                    paged::ServerPage::Offline => FetchResult {
                        has_more: false,
                        error: None,
                    },
                    paged::ServerPage::Error(e) => FetchResult {
                        has_more: false,
                        error: episodes.read().is_empty().then_some(e),
                    },
                }
            }))
        },
    );
    let PagedPool {
        page,
        visible,
        mut has_more,
        mut loading,
        mut error,
        refresh_nonce,
        initial_settled,
        ..
    } = pool;

    // Id-list body-fetch effect (playlist / downloads / history / OnDevice /
    // selected). Kept separate from the cache-query plan because it SUBSCRIBES to
    // EpisodeState — a playlist's membership, the user's playbacks, and the device set
    // all land via `do_pull` after the first run, so it must re-run when they arrive
    // to fetch their (not-yet-pooled) bodies. Loop-safe: `fetch_by_ids` runs only
    // when ids are genuinely missing, so once pooled the next run finds nothing and
    // stops — and (unlike the pool plan) it only flips `loading` when it actually
    // fetches, so an unrelated publish (e.g. download progress) doesn't flicker the
    // spinner. On success it bumps `cache_version` for effect A.
    use_effect(move || {
        if !paged {
            return;
        }
        let _ = refresh_nonce(); // subscribe: pull-to-refresh forces a revalidate
        let source = source_for_idlist.clone();
        // Subscribe to the gate only (memo); `.read()` the set ONLY while it's on.
        let selected_only = selected_only();
        let selected = if selected_only {
            ms.read().selected.clone()
        } else {
            std::collections::HashSet::new()
        };
        // Only the id-list/selected path runs here; cache-query goes through the pool.
        if !(id_list_mode() || selected_only) {
            return;
        }
        let id_window = {
            let app_ref = app_state_clone.read(); // id-list subscribes to EpisodeState
            let pl_ref = playlists_clone.read(); // …and to playlist membership (queue/playlists)
            let pb_ref = playbacks_clone.read(); // …and to the playbacks overlay (History)
            let dl_ref = downloads_clone.read(); // …and to the download set (OnDevice/Downloads)
            id_list_for_render(
                &source,
                filter.read().podcast_id,
                selected_only,
                &selected,
                id_list_mode(),
                &app_ref,
                &pl_ref,
                &pb_ref,
                &dl_ref,
            )
        };
        let Some(ids) = id_window else {
            return;
        };
        // Playlist sorts can surface ids beyond the first window, so fetch ALL its
        // missing bodies; window-only is fine for the intrinsically-ordered
        // ClientDownloads/History (and the bounded selected set).
        let fetch_all = !selected_only && matches!(source, ListSource::Playlist { .. });
        let vis = visible() as i32;
        let store = store_b.clone();
        let cfg = config.read();
        let deps = paged::PagedDeps {
            dispatch,
            server_url: cfg.server_url.clone(),
            token: cfg.access_token.clone(),
            offline: cfg.manual_offline,
        };
        drop(cfg);
        spawn(async move {
            // Gate concurrent duplicates: set the guard synchronously BEFORE the
            // first await so a sibling spawn (from a near-simultaneous publish) sees
            // it set and bails. Cleared on every exit path.
            if *idlist_inflight.peek() {
                return;
            }
            idlist_inflight.set(true);
            let Some(store) = store else {
                idlist_inflight.set(false);
                return;
            };
            let missing = if fetch_all {
                paged::id_all_missing(&store, &ids).await
            } else {
                paged::id_window_missing(&store, &ids, vis).await
            };
            if missing.is_empty() {
                idlist_inflight.set(false);
                return;
            }
            loading.set(true);
            error.set(None);
            match paged::fetch_by_ids(&deps, missing).await {
                paged::ServerPage::Fetched { .. } => cache_version += 1,
                paged::ServerPage::Offline => {}
                paged::ServerPage::Error(e) => {
                    if episodes.read().is_empty() {
                        error.set(Some(e));
                    }
                }
            }
            loading.set(false);
            idlist_inflight.set(false);
        });
    });

    // Whether ANY commit landed since mount: the async id-list resolve has no
    // loading/settling cover, so pre-commit frames would flash "No episodes found".
    let mut resolved_once = use_signal(|| false);

    // In-memory mode (paged = false): render the whole filtered set at once. The
    // pool's `visible` still drives how many of `episodes` are revealed on scroll.
    {
        let source_clone = source_for_inmem.clone();
        use_effect(move || {
            if paged {
                return;
            }
            let _ = refresh_nonce(); // subscribe: pull-to-refresh forces a reload
            let source = source_clone.clone();
            let sort_val = sort.read().clone();
            let filter_val = filter.read().clone();
            let app_state = app_state_clone();
            let playlists = playlists_clone();
            let playbacks = playbacks_clone();
            let downloads = downloads_clone();
            let podcasts = podcasts_clone();
            spawn(async move {
                load_page(
                    &source,
                    &sort_val,
                    &filter_val,
                    &mut episodes,
                    &mut pool_count,
                    &mut loading,
                    &mut has_more,
                    &mut error,
                    &app_state,
                    &playlists,
                    &playbacks,
                    &downloads,
                    &podcasts,
                )
                .await;
                if !*resolved_once.peek() {
                    resolved_once.set(true);
                }
            });
        });
    }

    // Paged effect A — render from the cache pool (filter + sort + `visible`
    // window). Re-runs on filter/sort/window change, on `cache_version`, and on any
    // EpisodeState change.
    //
    // SUBSCRIBES to EpisodeState. Id-list re-resolves on membership changes; cache-query
    // needs it because the fetched page is upserted by the *worker* (single store
    // writer) — and the worker may be busy in `do_pull` when the pool's plan
    // dispatches `CacheEpisodes`, so `cache_version` bumps *before* the upsert lands.
    // The worker `publish()`es right after upserting, so this EpisodeState subscription
    // is what re-runs us to read the now-fresh store. (No loop: effect A only reads
    // the store and sets local signals — it never writes EpisodeState or dispatches.)
    // Latest-wins guard for effect A's async store reads: each re-run (sort/filter/
    // window/pool-version change) claims a fresh generation, so an earlier in-flight
    // read that resolves out of order can't overwrite a newer render's window/count
    // (mirrors `use_paged_pool`'s `revalidate_gen`).
    let render_gen = use_latest_wins();
    // Empty-cache cold-load guard for the AllEpisodes cache-query first paint.
    // The cached pool paints IMMEDIATELY (cache-first, like the id-list pages) —
    // this only covers the case where the pool has NOTHING to paint while the
    // initial fetch is still landing: `loading` clears as soon as the HTTP future
    // resolves — before the worker's upsert+publish (and the settle sleep below)
    // make the rows queryable — so without this the render body would flash "No
    // episodes found" between the skeletons and the first real paint. Cleared
    // when a commit actually lands; the render body reads it to keep the
    // skeletons up and the empty state suppressed. (Written here, never read by
    // this effect, so flipping it can't re-run the effect.)
    let mut first_paint_settling = use_signal(|| false);
    // Long enough to outlast the worker's upsert+publish after the fetch future
    // resolves (the fetch dispatches the store write async); `render_gen` last-wins
    // means a publish arriving within the window just supersedes to a fresher read.
    const FIRST_PAINT_SETTLE_MS: u32 = 350;
    use_effect(move || {
        if !paged {
            return;
        }
        let g = render_gen.claim();
        // Subscribe to the pool's first-fetch settle signal: `false` while an
        // initial cold fetch is still in flight (online), `true` the moment the
        // cache is authoritative (offline / warm-fresh) or once that fetch lands.
        // Only consulted when the pool has NOTHING to paint (the empty-pool hold
        // in the AllEpisodes branch) — a non-empty cache always paints at once.
        let settled = initial_settled();
        let source = source_for_render.clone();
        let sort_val = sort.read().clone();
        let filter_val = filter.read().clone();
        let vis = visible() as i32;
        let _ = cache_version(); // subscribe: re-resolve after each upsert
        // Subscribe to the gate via its memo (re-runs on toggle); subscribe to the
        // selection set ONLY while it's on, so ticking a box doesn't re-query the
        // pool off the chip when the Selected view isn't even showing.
        let selected_only = selected_only();
        let selected = if selected_only {
            ms.read().selected.clone()
        } else {
            std::collections::HashSet::new()
        };
        let store = store_a.clone();
        // Subscribe to the five state slices — any of them can change the render:
        //  - EpisodeState: the fetched page is upserted by the worker (see header);
        //  - downloads: ClientDownloads/OnDevice membership + a download completing;
        //  - podcasts: search-by-show-name resolves titles from the pool;
        //  - playlists: the Playlist source's membership + queue changes;
        //  - playbacks: progress enrichment + History membership (cursor saves).
        // Subscription is the bare `read()`; the deep CLONES below are per branch,
        // so each publish pays only for what the taken branch consumes. Previously
        // all five were cloned unconditionally — the full EpisodeState (every cached
        // body) copied on every worker publish (~1 Hz per in-flight download), on
        // every mounted list, even though the hot AllEpisodes path never reads it.
        let _ = app_state_clone.read();
        let _ = downloads_clone.read();
        let _ = podcasts_clone.read();
        let _ = playlists_clone.read();
        let _ = playbacks_clone.read();
        // Every branch enriches rows with the playback overlay (small: id → cursor).
        let playbacks = playbacks_clone.read().clone();
        // The podcast pool's only consumer here is show-name search matching
        // (`read_cache` populates `podcast_names` / `apply_filter_sort` resolves
        // titles only under `filter.search`) — skip the pool copy otherwise.
        let podcasts = if filter_val.search.is_some() {
            podcasts_clone.read().clone()
        } else {
            PodcastState::default()
        };
        // Branch-conditional snapshots (`None` where the branch can't need them).
        let on_device = filter_val.filters.contains(&EpisodeFilter::OnDevice);
        // Derive the id-list predicates from `is_id_list()` (single source of
        // truth for the variant set — hand-enumerating it here is exactly how
        // `ServerDownloads` shipped with a missing snapshot and panicked):
        // the downloads/history-style branch is every id-list source EXCEPT
        // Playlist, which has its own arm with different needs.
        let downloads_history_like =
            source.is_id_list() && !matches!(&source, ListSource::Playlist { .. });
        let needs_app_state = selected_only
            || downloads_history_like
            || (matches!(&source, ListSource::AllEpisodes) && on_device);
        let app_state = needs_app_state.then(|| app_state_clone.read().clone());
        let needs_downloads = !selected_only && (on_device || downloads_history_like);
        let downloads = needs_downloads.then(|| downloads_clone.read().clone());
        let needs_playlists = !selected_only && source.is_id_list();
        let playlists = needs_playlists.then(|| playlists_clone.read().clone());
        spawn(async move {
            let Some(store) = store else { return };
            // The per-branch `expect`s below match the `needs_*` predicates above:
            // each branch unwraps only the snapshots its predicate included.
            // Commit a freshly-resolved row set to the render signals, but ONLY when
            // it actually differs from what's already shown. Effect A subscribes to
            // EpisodeState + playlists + playbacks + downloads + podcasts, so a bursty
            // worker publish (download-progress ticks, a playback-cursor save) re-runs
            // it even when the visible window is unchanged; an unconditional
            // `episodes.set` would then re-render the whole list (clone every row,
            // re-diff every `EpisodeListItem`) for nothing. The peek/compare is O(n)
            // over a windowed slice — far cheaper than the re-render it prevents.
            // Bind the comparison to a `let` so the `peek()` borrow is released before
            // `set()` (a borrow held across `set` would be a RefCell double-borrow).
            let mut commit_windowed = move |rows: Vec<EpisodeData>, vis: i32, ids_total: usize| {
                if !render_gen.is_current(g) {
                    return;
                }
                let count = rows.len();
                let windowed: Vec<EpisodeData> =
                    rows.into_iter().take(vis.max(0) as usize).collect();
                let count_changed = *pool_count.peek() != count;
                if count_changed {
                    pool_count.set(count);
                }
                // The unresolved membership size — the scroll gate's second axis
                // (see `id_total`'s declaration).
                if *id_total.peek() != ids_total {
                    id_total.set(ids_total);
                }
                let rows_changed = *episodes.peek() != windowed;
                if rows_changed {
                    episodes.set(windowed);
                }
                // A commit landed — the first-paint window (if any) is over. Cleared
                // here too (not just the cache-query branch) so a mode flip while
                // held (e.g. toggling OnDevice) can't leave the flag stuck.
                if *first_paint_settling.peek() {
                    first_paint_settling.set(false);
                }
                if !*resolved_once.peek() {
                    resolved_once.set(true);
                }
            };
            // "Selected" chip: render exactly the ticked set (scoped to this list's
            // podcast, if any), applying only search + the current sort — the
            // download/played chips are bypassed so ALL selected episodes show.
            if selected_only {
                let app_state = app_state.expect("selected branch snapshots app_state");
                let ids = id_list_for_selected(&selected, filter_val.podcast_id, &app_state);
                let all = paged::resolve_all(&store, &ids, &playbacks).await;
                let sel_filter = FilterSpec {
                    search: filter_val.search.clone(),
                    podcast_id: filter_val.podcast_id,
                    published_after: None,
                    filters: Vec::new(),
                };
                let rows =
                    apply_filter_sort(all, &sort_val, &sel_filter, None, &podcasts.podcasts_by_id);
                commit_windowed(rows, vis, ids.len());
                return;
            }
            match &source {
                // Playlist: resolve the WHOLE bounded membership, then filter+sort
                // (Custom = the raw Vec/position order via a rank map), then window.
                // This is what makes sort + filter actually work on queue/playlist.
                ListSource::Playlist { id } => {
                    let playlists = playlists.expect("playlist branch snapshots playlists");
                    let ids = playlists
                        .episodes_by_playlist
                        .get(id)
                        .cloned()
                        .unwrap_or_default();
                    let mut all = paged::resolve_all(&store, &ids, &playbacks).await;
                    // OnDevice is a client-only facet (device-download set), so it's
                    // applied here where EpisodeState is available — not in the pure
                    // `apply_filter_sort`.
                    if on_device {
                        let downloads = downloads.expect("OnDevice snapshots downloads");
                        all.retain(|ep| {
                            downloads.device_state(ep.id) == Some(ClientDownloadState::Downloaded)
                        });
                    }
                    let rank: std::collections::HashMap<i32, usize> =
                        ids.iter().enumerate().map(|(i, id)| (*id, i)).collect();
                    let rows = apply_filter_sort(
                        all,
                        &sort_val,
                        &filter_val,
                        Some(&rank),
                        &podcasts.podcasts_by_id,
                    );
                    // Filtered length drives infinite-scroll's stop condition.
                    commit_windowed(rows, vis, ids.len());
                }
                // History / Downloads: resolve the WHOLE intrinsic membership, then
                // filter + sort under the current predicate (search, played chips,
                // sort field) before windowing — mirroring the Playlist branch, so
                // these controls aren't silently ignored. The OnDevice facet trims
                // the id list (to fully-downloaded) before resolving.
                ListSource::ClientDownloads | ListSource::ServerDownloads | ListSource::History => {
                    let app_state =
                        app_state.expect("downloads/history branch snapshots app_state");
                    let playlists =
                        playlists.expect("downloads/history branch snapshots playlists");
                    let downloads =
                        downloads.expect("downloads/history branch snapshots downloads");
                    let mut ids =
                        id_list_for_source(&source, &app_state, &playlists, &playbacks, &downloads)
                            .unwrap_or_default();
                    if on_device {
                        ids.retain(|id| {
                            downloads.device_state(*id) == Some(ClientDownloadState::Downloaded)
                        });
                    }
                    let all = paged::resolve_all(&store, &ids, &playbacks).await;
                    // The intrinsic order (History: play-recency desc; Downloads: id
                    // desc) is the position rank — `Custom` keeps it, any explicit
                    // sort overrides it.
                    let rank: std::collections::HashMap<i32, usize> =
                        ids.iter().enumerate().map(|(i, id)| (*id, i)).collect();
                    // History's default sort is the non-selectable `UpdatedAt`
                    // sentinel meaning "natural order"; map it to the rank so
                    // recent-played-first stays the default (the episode body's
                    // `updated_at` isn't play recency). Downloads' default
                    // (Published) is a real, selectable sort, so it's left alone.
                    let sort_eff = if matches!(source, ListSource::History)
                        && sort_val.field == SortField::UpdatedAt
                    {
                        SortSpec {
                            field: SortField::Custom,
                            direction: OrderDirection::Asc,
                        }
                    } else {
                        sort_val.clone()
                    };
                    let rows = apply_filter_sort(
                        all,
                        &sort_eff,
                        &filter_val,
                        Some(&rank),
                        &podcasts.podcasts_by_id,
                    );
                    // Filtered length drives infinite-scroll's stop condition.
                    commit_windowed(rows, vis, ids.len());
                }
                ListSource::AllEpisodes => {
                    if on_device {
                        // OnDevice → render the local device set (downloaded only),
                        // scoped to this podcast if the list is. Search / Played /
                        // Unplayed still apply client-side over this small set. The
                        // device id set IS the OnDevice filter, so the token is a
                        // no-op inside `apply_filter_sort` (it ignores OnDevice).
                        let app_state = app_state.expect("OnDevice branch snapshots app_state");
                        let downloads = downloads.expect("OnDevice branch snapshots downloads");
                        let ids = id_list_for_device(&app_state, &downloads, filter_val.podcast_id);
                        let all = paged::resolve_all(&store, &ids, &playbacks).await;
                        let rows = apply_filter_sort(
                            all,
                            &sort_val,
                            &filter_val,
                            None,
                            &podcasts.podcasts_by_id,
                        );
                        commit_windowed(rows, vis, ids.len());
                    } else {
                        // Cache-first (stale-while-revalidate): query the pool by
                        // filter + sort and paint whatever it already holds, exactly
                        // like the id-list pages — navigating to /latest renders the
                        // cached feed instantly while the pool's revalidate fetch runs
                        // in the background. The fetch upserts via the worker, whose
                        // publish re-runs this effect; the diff-guard below then
                        // commits only a real change (e.g. new episodes at the top).
                        // (This replaced a hold-skeletons-until-the-fetch-lands gate
                        // that made /latest feel slow on every navigation past the
                        // freshness TTL.)
                        let (mut rows, mut count) = paged::read_cache(
                            &store,
                            &sort_val,
                            &filter_val,
                            vis,
                            &playbacks,
                            &podcasts,
                        )
                        .await;
                        if !render_gen.is_current(g) {
                            return;
                        }
                        // Only a genuinely cold load with an EMPTY pool still waits:
                        // there's nothing cached to paint, and committing the empty
                        // set would flash "No episodes found" under the skeletons
                        // while the initial fetch is in flight. `settling` keeps the
                        // skeletons up (see the render body); the `settled` flip /
                        // worker publish re-runs this effect to paint the fresh page.
                        if rows.is_empty() && !settled {
                            if !*first_paint_settling.peek() {
                                first_paint_settling.set(true);
                            }
                            return;
                        }
                        // Releasing a held-empty first paint: the fetch has resolved
                        // but its rows may still be upserting in the worker — settle
                        // briefly and re-read so the first paint is the FRESH page,
                        // not an empty flash. A publish inside the window just
                        // supersedes this read (render_gen last-wins).
                        if rows.is_empty() && *first_paint_settling.peek() {
                            halogen_ui_platform::time::sleep_ms(FIRST_PAINT_SETTLE_MS).await;
                            if !render_gen.is_current(g) {
                                return;
                            }
                            (rows, count) = paged::read_cache(
                                &store,
                                &sort_val,
                                &filter_val,
                                vis,
                                &playbacks,
                                &podcasts,
                            )
                            .await;
                        }
                        if render_gen.is_current(g) {
                            // Same diff-guard as `commit_windowed`; rows are
                            // already windowed by `read_cache`, count is separate.
                            let count_changed = *pool_count.peek() != count;
                            if count_changed {
                                pool_count.set(count);
                            }
                            let rows_changed = *episodes.peek() != rows;
                            if rows_changed {
                                episodes.set(rows);
                            }
                            if *first_paint_settling.peek() {
                                first_paint_settling.set(false);
                            }
                            if !*resolved_once.peek() {
                                resolved_once.set(true);
                            }
                        }
                    }
                }
            }
        });
    });

    // Narrowed selector over the session token (`server_url`): the history reset-load
    // below must re-fire when auth lands, but `config.read()` would subscribe it to
    // the WHOLE `ClientConfig`, so any unrelated mutation (offline toggle, theme,
    // prefs) would re-fire `load_history(reset=true)` and jump History back to page 0.
    // A memo only notifies when the token actually changes — exactly when a reset is
    // wanted.
    let history_auth = use_memo(move || config.read().server_url.clone());

    // History paging effect 1 — (re)start the `/playbacks` paging on mount, on each
    // pull-to-refresh (`refresh_nonce`), when the session lands (the `history_auth`
    // selector re-fires once login sets the token — mirrors the cache-query plan,
    // minus the whole-config churn), and when connectivity RETURNS (the offline
    // memo flips). The worker merges page 0 into the overlay; effect A then sorts
    // it into the History id list. `reset` also clears the
    // `history_has_more`/`history_next_page` cursor state.
    let is_offline = use_is_offline();
    use_effect(move || {
        if !is_history {
            return;
        }
        let _ = refresh_nonce(); // mount + each pull-to-refresh
        let _ = history_auth(); // re-fire only when the token (auth) actually lands
        // Offline: don't issue the load OR set the in-flight guard — the worker's
        // `load_history` early-returns without advancing the cursor, which would
        // latch `history_inflight` forever (effect 2 clears it only on a cursor
        // advance). Subscribing to the offline memo re-fires this exactly when
        // connectivity returns, so a History page mounted offline recovers on its
        // own (the reset also re-arms `history_has_more` after a failed fetch).
        if is_offline() {
            return;
        }
        history_inflight.set(true);
        halogen_ui_state::commands::load_history(&dispatch, true);
    });

    // History paging effect 2 — page ahead as the render window nears the end of the
    // loaded ids. Gated on `history_has_more` and `next_page > 0` (i.e. the initial
    // reset load has landed), and deduped via `history_inflight`, cleared when the
    // worker advances `history_next_page` (a page landed). The guard signals are
    // `peek`ed (not subscribed): the effect re-runs on real state — app_state /
    // window growth — not on its own bookkeeping writes, so it can't self-loop.
    use_effect(move || {
        if !is_history {
            return;
        }
        let h = history.read();
        let next_page = h.history_next_page;
        let has_more = h.history_has_more;
        drop(h);
        // Gate page-ahead on the count of ids actually LOADED from the server
        // (unfiltered), not the filter-trimmed `pool_count`: a filter chip shrinks
        // `pool_count` while the server still has more pages, which would otherwise
        // make `count - vis` fire continuously (over-fetch) or never (stall).
        // History membership is the playbacks overlay, not the episode pool, the
        // download set, or the playlist pool, so `peek` those (don't subscribe this
        // page-ahead effect to their churn — it re-runs on the history cursor above).
        let loaded = id_list_for_source(
            &source_for_history,
            &app_state.peek(),
            &playlists.peek(),
            &playbacks.peek(),
            &downloads.peek(),
        )
        .map(|v| v.len())
        .unwrap_or(0) as i32;
        // A page landed (cursor advanced) → clear the in-flight guard.
        if next_page != *history_seen_page.peek() {
            history_seen_page.set(next_page);
            history_inflight.set(false);
        }
        let vis = visible() as i32;
        if has_more && next_page > 0 && !*history_inflight.peek() && loaded - vis < paged::PAGE_SIZE
        {
            history_inflight.set(true);
            halogen_ui_state::commands::load_history(&dispatch, false);
        }
    });

    // Sort/filter change: reset the server cursor + render window via the pool, but
    // keep the pool's rows — paged effect A re-queries them under the new predicate
    // instantly; the in-memory effect reloads. (The pool is never cleared, so
    // toggling a filter off shows everything again.)
    let on_sort_change = move |new_sort: SortSpec| {
        sort.set(new_sort);
        pool.reset();
    };

    let on_filter_change = move |new_filter: FilterSpec| {
        filter.set(new_filter);
        pool.reset();
    };

    // Pull-to-refresh. On a podcast-detail list the scope is the filter's
    // `podcast_id` (poll just that feed); elsewhere it's `None` (poll all). The
    // refresh resets the paging cursors + bumps the nonce (force a re-fetch even at
    // page 0) and asks the worker for a full pull so id-list memberships
    // (queue/playlists) reconcile too.
    let pull_scope = use_memo(move || filter.read().podcast_id);
    let on_refresh = use_callback(move |_: ()| {
        pool.refresh();
        halogen_ui_state::commands::refresh(&dispatch);
    });
    let is_admin = use_is_admin();
    let pull = use_pull_to_refresh("episode-scroll", pull_scope, on_refresh);

    // Scroll-position memory (opt-in via `scroll_key`). Restores the render window
    // + offset on return; the stamp invalidates the offset across sort/filter
    // changes. No-op when `scroll_key` is None.
    let scroll_stamp = use_memo(move || list_stamp(&sort.read(), &filter.read()));
    use_scroll_memory(
        scroll_key,
        "episode-scroll",
        ScrollPaging {
            visible,
            page,
            has_more,
        },
        scroll_stamp,
    );

    // Last-rendered-rows memory (same opt-in key): paints the previous visit's
    // rows on frame one; the resolve effects above then revalidate + diff-commit.
    // Stamp includes the SOURCE: the "queue" key survives a default-playlist swap.
    let row_stamp = use_memo(move || {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        list_stamp(&sort.read(), &filter.read()).hash(&mut h);
        format!("{:?}", source_for_rowmem).hash(&mut h);
        h.finish()
    });
    use_row_memory(
        scroll_key.or(rows_key),
        row_stamp,
        episodes,
        pool_count,
        id_total,
    );

    // Infinite scroll (EpisodeList drives its own — see `use_paged_pool_with(None)`).
    // A persistent 250ms sentinel trigger grows the render window and, for the
    // cache-query mode, advances the server cursor. Crucially it stops growing once
    // the whole pool is shown (`visible >= pool_count` with no more server pages),
    // so parking at the bottom of a fully-loaded list doesn't churn effect A's store
    // re-query every tick. Id-list mode is bounded by its id count (no page cursor);
    // in-memory mode just reveals 30 more loaded rows.
    let mut page = page;
    let mut visible = visible;
    use_dom_stream(
        || infinite_scroll_body("episode-scroll", "episode-sentinel"),
        move |_: bool| {
            if paged {
                let more = if id_list_mode() {
                    // Grow past the RESOLVED rows up to the membership size, so
                    // missing bodies past the window get fetched (see `id_total`).
                    visible() < pool_count().max(id_total())
                } else {
                    has_more() || visible() < pool_count()
                };
                if more && !loading() {
                    visible += paged::PAGE_SIZE as usize;
                    if !id_list_mode() && has_more() {
                        page += 1;
                    }
                }
            } else if visible() < episodes.read().len() {
                visible += 30;
            }
        },
    );

    // Settling counts as loading for presentation: the empty state must never
    // flash mid-fetch (`first_paint_settling`) or before the first commit lands.
    let settling = first_paint_settling() || !resolved_once();
    let show_empty = episodes.is_empty() && !loading() && !settling;

    let swipe_config = swipe;

    // Multiselect snapshots for this render: whether checkboxes show, and the
    // selected set (so each row knows if it's ticked). Reading `ms` here subscribes
    // the component, so toggling re-renders the list (and the affected rows).
    let ms_active = ms.read().active;
    let selected_set = ms.read().selected.clone();

    // ONE stable selection-toggle callback for every row, keyed by episode id passed
    // at call time. A fresh `Callback::new` per row each render is NOT prop-stable
    // (closures aren't `PartialEq`), so it made Dioxus treat every row's
    // `on_toggle_select` prop as "changed" and re-render the whole rendered set on any
    // parent re-render (window grew on scroll, multiselect toggled, …) — defeating the
    // row's PartialEq memos. `use_callback` is stable across renders, so unchanged
    // rows now skip re-render. Toggle: `insert` returns false when already present.
    let on_toggle_select = use_callback(move |episode_id: i32| {
        let mut s = ms.write();
        if !s.selected.insert(episode_id) {
            s.selected.remove(&episode_id);
        }
    });

    // Select every currently-LOADED row (the fetched/filtered set — not the whole
    // server list). Additive: keeps prior manual ticks; rows loaded later are not
    // auto-selected.
    let on_select_all = use_callback(move |_: ()| {
        let ids: Vec<i32> = episodes.read().iter().map(|ep| ep.id).collect();
        ms.write().selected.extend(ids);
    });

    // Paged mode already holds exactly the fetched rows; in-memory mode virtualizes
    // by revealing `visible` of the full set.
    let take_n = if paged { usize::MAX } else { visible() };
    let episode_items: Vec<_> = episodes
        .read()
        .iter()
        .take(take_n)
        .map(|ep| {
            let episode_id = ep.id;
            let sw = swipe_config;
            let progress = compute_progress(ep, &item_variant);
            let is_selected = selected_set.contains(&episode_id);
            (ep.clone(), progress, sw, is_selected)
        })
        .collect();

    // Manual reorder is available only on a playlist list in Custom order — and
    // never while multiselect is active (the grip is replaced by the checkbox) or
    // while a filter/search narrows the rendered set. The reorder target is a
    // rendered-row index applied directly to the stored membership, so with rows
    // hidden it would land at the wrong stored position — disable it instead of
    // corrupting the order (the user clears the filter to reorder).
    let playlist_id = match &source {
        ListSource::Playlist { id } => Some(*id),
        _ => None,
    };
    let list_len = episode_items.len();
    // Reorder targets are rendered-row indices, valid only when the rendered
    // rows are a gap-free, in-order image of the stored membership. `resolve_all`
    // silently drops members whose bodies aren't cached, so a mid-list gap
    // shifts every later row's index off its true stored position (and the
    // descending mirror axis, `full_len = pool_count`, undercounts too). Require
    // every member RESOLVED: pool count == membership length. The render WINDOW
    // being smaller than that is fine — it's a strict prefix of the displayed
    // order, so within-window indices are exact and the descending mirror keys
    // off pool_count, not the window. (Gating on rendered count too made the
    // grip vanish on any list longer than one scroll window.) Unresolved
    // members disable the grip until scrolling fetches them (NH13 grows the
    // window to do that) rather than move to a wrong slot.
    let membership_len = playlist_id
        .and_then(|id| {
            playlists
                .read()
                .episodes_by_playlist
                .get(&id)
                .map(|v| v.len())
        })
        .unwrap_or(0);
    let fully_resolved = membership_len > 0 && pool_count() == membership_len;
    let reorder_enabled = playlist_id.is_some()
        && sort.read().field == SortField::Custom
        && !ms_active
        && !filter.read().is_narrowing()
        && fully_resolved;
    // Custom order is stored ascending; a descending view (down arrow) renders it
    // reversed, so reorder targets must be mirrored back to stored positions. The
    // mirror axis is the full (un-windowed) pool length, not the rendered window.
    let reversed = sort.read().direction == OrderDirection::Desc;
    let full_len = pool_count();

    // Open the bulk-action menu (reuses the shared QuickContextMenu). Snapshots the
    // current selection plus the queue / current-playlist context; the worker emits
    // the result toast, and the menu host closes the panel on select.
    let on_open_menu = use_callback(move |_: ()| {
        let ids: Vec<i32> = ms.read().selected.iter().copied().collect();
        if ids.is_empty() {
            toast.info("Select some episodes first.");
            return;
        }
        let n = ids.len();
        let queue_id = playlists.read().queue_id();
        quick.open(
            format!("{n} selected"),
            bulk_menu_sections(
                ids,
                dispatch,
                nav,
                queue_id,
                playlist_id,
                config.read().server_kind.is_embedded(),
            ),
        );
    });

    rsx!(
        div { class: "flex flex-col h-full",
            ListControls {
                embedded: config.read().server_kind.is_embedded(),
                source: source.clone(),
                sort,
                filter,
                on_sort_change,
                on_filter_change,
                ms,
                on_open_menu,
                on_select_all,
            }

            // Positioning context for the pull-to-refresh overlay. The indicator
            // sits OUTSIDE the scroll container (absolute overlay) so a background
            // sync/poll can't push the whole list down — that in-flow shove was a
            // top CLS source. `min-h-0` lets the scroll child shrink to scroll.
            div { class: "relative flex-1 min-h-0",
                PullToRefreshIndicator { phase: pull.phase, is_admin: is_admin() }

                div {
                    id: "episode-scroll",
                    class: "h-full overflow-y-auto overscroll-y-contain",
                    // Read by the pull-to-refresh JS shim: while multiselect is active it
                    // ignores the gesture (no pull/poll mid-selection).
                    "data-multiselect": if ms_active { "1" } else { "0" },

                    if let Some(msg) = error() {
                        div { class: "p-4 text-center text-error",
                            "Error loading episodes: {msg}"
                        }
                    }

                if show_empty {
                    div { class: "p-8 text-center text-muted",
                        "No episodes found"
                    }
                }

                for (idx, item) in episode_items.iter().enumerate() {
                    EpisodeListItem {
                        key: "{item.0.id}",
                        episode: item.0.clone(),
                        progress: item.1,
                        swipe: item.2,
                        reorder_enabled,
                        position: idx,
                        list_len,
                        reversed,
                        full_len,
                        playlist_id,
                        multiselect_active: ms_active,
                        selected: item.3,
                        on_toggle_select,
                    }
                }

                // Cold load: reserve ~a screen of height with skeleton rows so the
                // list doesn't jump when the first page lands (CLS). Append-more
                // (rows already present) uses the bottom overlay spinner below, which
                // is out of flow so paging can't collapse the layout.
                if (loading() || settling) && episode_items.is_empty() {
                    for i in 0..8 {
                        // One real row's height (8.9rem, flush — no wrapper padding) so
                        // the skeleton→content swap is height-neutral (no CLS). Must
                        // track `EpisodeListItem`'s `contain-intrinsic-size`.
                        div {
                            key: "skel-{i}",
                            class: "skeleton w-full rounded-lg",
                            style: "height: 8.9rem;",
                        }
                    }
                }

                    div {
                        id: "episode-sentinel",
                        class: "h-4",
                    }
                }

                // Append-more spinner: an ABSOLUTE overlay pinned to the bottom of the
                // scroll area (out of flow), so it appearing/vanishing as infinite-scroll
                // pages load can't collapse the layout and shift content (a scroll-time
                // CLS). Cold load uses the in-flow skeletons above; this shows only when
                // paging an already-populated list.
                if loading() && !episode_items.is_empty() {
                    div {
                        class: "absolute bottom-0 inset-x-0 flex justify-center p-2 pointer-events-none",
                        span { class: "loading loading-spinner loading-sm" }
                    }
                }
            }
        }
    )
}

async fn load_page(
    source: &ListSource,
    sort: &SortSpec,
    filter: &FilterSpec,
    episodes: &mut Signal<Vec<EpisodeData>>,
    pool_count: &mut Signal<usize>,
    loading: &mut Signal<bool>,
    has_more: &mut Signal<bool>,
    error: &mut Signal<Option<String>>,
    app_state: &EpisodeState,
    playlists: &PlaylistState,
    playbacks: &PlaybackState,
    downloads: &DownloadState,
    podcasts: &PodcastState,
) {
    loading.set(true);
    error.set(None);

    debug!(
        "Loading source {:?} with sort {:?} and filter {:?}",
        source, sort, filter
    );

    // Pick the base set of episodes for this source from the worker-owned state.
    // Playlist/Queue read their membership; ClientDownloads filters all episodes
    // to the device-download set; everything else reads all episodes.
    let base: Vec<halogen_wire::EpisodeData> = match source {
        ListSource::Playlist { id } => playlists.playlist_episodes(app_state, *id),
        ListSource::ClientDownloads => app_state
            .episodes_by_id
            .values()
            .filter(|ep| downloads.client_downloads.contains_key(&ep.id))
            .cloned()
            .collect(),
        // Embedded-mode Downloads: the SERVER's download set (mirrors
        // `id_list_for_source` — downloading items visible, failures live on
        // the server-errors page).
        ListSource::ServerDownloads => app_state
            .episodes_by_id
            .values()
            .filter(|ep| {
                matches!(
                    ep.download_status,
                    halogen_wire::DownloadStatus::Downloading
                        | halogen_wire::DownloadStatus::Downloaded
                )
            })
            .cloned()
            .collect(),
        _ => app_state.episodes_by_id.values().cloned().collect(),
    };

    // Attach each episode's playback (progress / unplayed / History): the
    // optimistic overlay wins, else the cursor the server embedded on the body.
    let all_eps: Vec<EpisodeData> = base
        .into_iter()
        .map(|ep| paged::enrich_playback(ep, &playbacks.playbacks))
        .collect();

    // History keeps only episodes that have a playback record.
    let all_eps: Vec<EpisodeData> = match source {
        ListSource::History => all_eps
            .into_iter()
            .filter(|e| e.playback.is_some())
            .collect(),
        _ => all_eps,
    };

    let sorted = apply_filter_sort(all_eps, sort, filter, None, &podcasts.podcasts_by_id);

    // All data is already in memory; render the full filtered/sorted set and let
    // the locked scroll container handle scrolling. (Server-side paging/virtualization
    // can be layered back in when lists outgrow memory.)
    // Set the pool size too (the reorder mirror axis + scroll gate read it); the
    // paged path sets it, so the in-memory path must as well, else `full_len` is 0.
    pool_count.set(sorted.len());
    episodes.set(sorted);
    has_more.set(false);
    loading.set(false);
}
