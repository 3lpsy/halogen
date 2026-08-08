use dioxus::prelude::*;
use halogen_wire::{DefaultListParams, Pagination, PodcastData, PodcastInclude};

use super::controls::PodcastListControls;
use super::filter::apply_podcast_search_sort;
use crate::Route;
use crate::components::{
    Artwork, FilterSpec, OrderDirection, PullToRefreshIndicator, SortField, SortSpec,
    podcast_menu_sections, use_confirm, use_quick_menu,
};
use crate::pages::PagedListPage;
use halogen_ui_appstate::media_url;
use halogen_ui_icons::ChevronRight;
use halogen_ui_state::commands;
use halogen_ui_state::hooks::{
    PAGE_SIZE, PagedPool, paged_list_plan, use_config, use_dispatch, use_is_admin,
    use_list_view_state, use_paged_pool, use_paged_scroll_memory, use_podcasts,
    use_pull_to_refresh,
};

/// Podcasts page — subscribe to a feed and list every subscribed podcast,
/// **offline-first and paged**: render a window over the cached pool
/// (`podcasts_by_id`, hydrated from the local store on boot), and revalidate it
/// page-by-page from the server. Rendering subscribes to the pool, so the
/// worker's `CachePodcasts` upsert re-renders us — no store-read race. Each card
/// links to the podcast detail page.
#[component]
pub fn Podcasts() -> Element {
    let podcasts = use_podcasts();
    let dispatch = use_dispatch();
    let config = use_config();
    let quick = use_quick_menu();
    let confirm = use_confirm();
    let nav = use_navigator();

    // Sort + search, remembered across navigation + restart (URL query + sticky
    // config). Default: Title ascending. Applied client-side over the cached pool.
    let (mut sort, mut filter) = use_list_view_state(
        "podcasts",
        SortSpec {
            field: SortField::Title,
            direction: OrderDirection::Asc,
        },
        FilterSpec::default(),
    );

    // Offline-first paged pool: render a window over the cached podcast pool and
    // revalidate it page-by-page (the worker upserts each fetched page into
    // `podcasts_by_id` + the store and publishes, re-rendering us). The fetch plan
    // has no reactive inputs beyond the cursor/config; sort + search are applied
    // client-side over the pool below.
    let pool = use_paged_pool(
        "podcast-scroll",
        "podcast-sentinel",
        // Bound window growth to the cached pool size so a short list (fewer podcasts
        // than fill the viewport) can't grow `visible` forever off the 250ms trigger.
        move || podcasts.peek().podcasts_by_id.len(),
        paged_list_plan(
            config,
            move || podcasts.peek().podcasts_by_id.is_empty(),
            move |data| commands::cache_podcasts(&dispatch, data),
            move |client, server_page| async move {
                let params = DefaultListParams::<PodcastInclude> {
                    pagination: Some(Pagination {
                        page: server_page,
                        size: PAGE_SIZE,
                    }),
                    ..Default::default()
                };
                client.list_podcasts(params).await.map(|resp| resp.data)
            },
        ),
    );
    let PagedPool {
        visible,
        loading,
        error,
        ..
    } = pool;

    // Remember scroll position + render window across navigation (in-memory, this
    // session). Restores the window so prior rows repaint from the cached pool,
    // then the offset; the stamp invalidates the offset across sort/search changes.
    use_paged_scroll_memory("podcasts", "podcast-scroll", pool, sort, filter);

    // Pull-to-refresh: reset the cursor, force a re-fetch, and ask the worker for
    // a full pull. Server poll (hold) is unscoped — the podcasts list spans feeds.
    let pull_scope = use_memo(|| None::<i32>);
    let on_refresh = use_callback(move |_: ()| {
        pool.refresh();
        commands::refresh(&dispatch);
    });
    let is_admin = use_is_admin();
    let pull = use_pull_to_refresh("podcast-scroll", pull_scope, on_refresh);

    // Sort/search change: just update the persisted signals. Filtering + sorting
    // run client-side over the cached pool, so there's no server cursor to reset.
    let on_sort_change = use_callback(move |new_sort: SortSpec| sort.set(new_sort));
    let on_filter_change = use_callback(move |new_filter: FilterSpec| filter.set(new_filter));

    // Render the pool, subscribing to EpisodeState so a `CachePodcasts` upsert
    // re-renders. Search + sort are applied client-side (the set is small and held
    // in memory). When a search is active we show every match (the pool is tiny);
    // otherwise we keep the infinite-scroll window over the browse order.
    let rows: Vec<PodcastData> = podcasts.read().podcasts_by_id.values().cloned().collect();
    let searching = filter.read().search.as_ref().is_some_and(|s| !s.is_empty());
    let mut rows = apply_podcast_search_sort(rows, &sort.read(), &filter.read());
    if !searching {
        rows.truncate(visible());
    }

    let empty_message = if searching {
        "No podcasts match your search."
    } else {
        "No podcasts yet — add one from Settings."
    };

    rsx! {
    // Bare, full-width framing to match the episode-list pages (latest/queue/
    // history/downloads): no page padding or in-page title (the nav names the
    // page) — just the controls bar over a full-bleed, flush card list. The
    // shared `PagedListPage` owns the frame, scroll container, error/empty
    // boilerplate and the bottom sentinel; this page keeps the data wiring,
    // pull-to-refresh, and the per-row card markup.
    PagedListPage {
        id_prefix: "podcast",
        error,
        loading,
        is_empty: rows.is_empty(),
        error_label: "podcasts",
        empty_message: empty_message.to_string(),
        controls: rsx! {
            PodcastListControls {
                sort,
                filter,
                on_sort_change,
                on_filter_change,
            }
        },
        pull: rsx! {
            PullToRefreshIndicator { phase: pull.phase, is_admin: is_admin() }
        },
        for p in rows {
            {
                            let count = p.episode_count.unwrap_or(0);
                            let pid = p.id;
                            let ptitle = p.title.clone();
                            // Shared podcast-actions menu (same as the detail
                            // header kebab); Delete opens this page's confirm.
                            let sections = podcast_menu_sections(
                                pid,
                                p.podcast_config_id,
                                nav,
                                confirm.purge_podcast_callback(pid),
                                confirm.delete_podcast_callback(pid),
                            );
                            rsx! {
                                // Clickable card via a stretched Link (an <a>):
                                // keyboard-focusable + SPA nav, and its `after:inset-0`
                                // pseudo-element covers the whole `relative` card so a
                                // tap anywhere still navigates. The kebab is a sibling
                                // raised above the overlay (`relative z-10`) so it stays
                                // independently clickable — no nested interactive.
                                div {
                                    key: "{pid}",
                                    class: "card bg-base-100 shadow-sm border border-base-300 hover:bg-base-200 transition-colors cursor-pointer relative",
                                    // Browser-native list virtualization, matching the episode
                                    // rows (see `EpisodeListItem`): off-screen cards skip render +
                                    // layout (cuts Style&Layout / paint on a long list) while
                                    // staying in the DOM so scroll-memory + infinite-scroll keep
                                    // working. The placeholder is in `rem`, NOT `px`: the root
                                    // font-size is user-scaled, so a px constant would under-reserve
                                    // by that factor and force a downward height correction on every
                                    // card scrolled past above the viewport — summed, that was a top
                                    // CLS source. The card is one deterministic height (fixed art +
                                    // truncated single-line text), so `5.1rem` matches it exactly and
                                    // `auto` remembers each card's real height after first paint.
                                    style: "content-visibility: auto; contain-intrinsic-size: auto 5.1rem;",
                                    div { class: "card-body p-3",
                                        div { class: "flex items-center gap-3",
                                            div { class: "flex-shrink-0 w-14 h-14 rounded bg-base-200 flex items-center justify-center overflow-hidden",
                                                // Server art cache (optimistic; placeholder on miss).
                                                // ~70px tile → the downscaled thumbnail.
                                                Artwork {
                                                    src: media_url::art_url_for_podcast_small(config.read().server_url.as_deref(), &p),
                                                    alt: "Podcast art",
                                                    img_class: "w-full h-full object-cover",
                                                    placeholder_class: "text-xl",
                                                }
                                            }
                                            div { class: "min-w-0 flex-1",
                                                h2 { class: "text-sm font-semibold leading-snug truncate",
                                                    Link {
                                                        to: Route::PodcastDetail { id: pid },
                                                        class: "after:absolute after:inset-0",
                                                        "{p.title}"
                                                    }
                                                }
                                                if let Some(author) = p.author.clone() {
                                                    p { class: "text-xs text-muted truncate mt-0.5", "{author}" }
                                                }
                                                p { class: "text-xs text-muted mt-0.5", "{count} episodes" }
                                            }
                                            // Kebab → shared podcast actions. `relative z-10`
                                            // lifts it above the card's stretched link overlay.
                                            button {
                                                class: "relative z-10 flex items-center justify-center w-9 h-9 shrink-0 text-lg leading-none rounded text-muted hover:text-base-content hover:bg-base-300",
                                                "aria-label": "Podcast actions",
                                                onclick: move |e: MouseEvent| {
                                                    e.stop_propagation();
                                                    quick.open(ptitle.clone(), sections.clone());
                                                },
                                                "⋯"
                                            }
                                            ChevronRight { class: "w-5 h-5 text-base-content/30 shrink-0" }
                                        }
                                    }
                                }
                            }
                        }
                    }
        }
    }
}
