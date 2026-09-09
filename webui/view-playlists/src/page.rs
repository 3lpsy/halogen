use dioxus::prelude::*;
use halogen_wire::{
    DefaultListParams, FilterParams, Order, Pagination, PlaylistData, PlaylistInclude,
};

use super::controls::PlaylistListControls;
use super::filter::apply_playlist_search_sort;
use crate::components::{
    FilterSpec, KebabButton, OrderDirection, SortField, SortSpec, playlist_menu_sections,
    start_drag_reorder, use_confirm, use_quick_menu,
};
use crate::pages::PagedListPage;
use halogen_webui_commands::actions as commands;
use halogen_webui_component_icons::GripVertical;
use halogen_webui_hooks::{
    PAGE_SIZE, PagedPool, paged_list_plan, use_config, use_connection, use_dispatch,
    use_list_view_state, use_paged_pool, use_paged_scroll_memory, use_playlists,
};

/// Map a UI sort field to the server `order_by` token.
fn order_by_token(field: SortField) -> &'static str {
    match field {
        SortField::Custom => "position",
        SortField::CreatedAt => "created_at",
        // Title carries the playlist name.
        _ => "name",
    }
}

fn db_direction(dir: OrderDirection) -> halogen_wire::OrderDirection {
    match dir {
        OrderDirection::Asc => halogen_wire::OrderDirection::Asc,
        OrderDirection::Desc => halogen_wire::OrderDirection::Desc,
    }
}

/// Playlists page, **offline-first and paged**, mirroring the podcasts list: render a window over the cached playlist
/// pool and revalidate it page-by-page from the server. Defaults to the manual "Custom" (`position`) order, which is
/// also the stable key the server pages by; Name/Created are selectable. Search is server-backed (typing fetches
/// matching playlists, not filtering the pool). Under Custom order the cards are drag-to-reorder.
#[component]
pub fn Playlists() -> Element {
    let playlists = use_playlists();
    let dispatch = use_dispatch();
    let config = use_config();
    let nav = use_navigator();
    let quick = use_quick_menu();
    let confirm = use_confirm();

    // Sort + search, remembered across navigation + restart. Default: Custom asc
    // (the manual position order).
    let (mut sort, mut filter) = use_list_view_state(
        "playlists",
        SortSpec {
            field: SortField::Custom,
            direction: OrderDirection::Asc,
        },
        FilterSpec::default(),
    );

    // Offline-first paged pool: render a window over the cached playlist pool and revalidate it page-by-page. The plan
    // reads `sort` + `search` synchronously, so the fetch re-runs when the order/filter changes (server-side order +
    // name filter) and a search lazily pulls matching playlists. The worker upserts via `CachePlaylists` and publishes,
    // re-rendering us.
    let pool = use_paged_pool(
        "playlist-scroll",
        "playlist-sentinel",
        // Bound window growth to the cached pool size so a short list can't grow
        // `visible` forever off the 250ms infinite-scroll trigger.
        move || playlists.peek().playlists.len(),
        paged_list_plan(
            config,
            move || playlists.peek().playlists.is_empty(),
            move |data| commands::cache_playlists(&dispatch, data),
            move |client, server_page| {
                // Reactive sort/search reads stay synchronous (outside the future) so
                // the pool re-fetches when the order/filter changes.
                let spec = sort.read().clone();
                let search = filter.read().search.clone();
                async move {
                    let filter_params =
                        search
                            .as_ref()
                            .filter(|s| !s.is_empty())
                            .map(|s| FilterParams {
                                search: Some(s.clone()),
                                ..Default::default()
                            });
                    let params = DefaultListParams::<PlaylistInclude> {
                        pagination: Some(Pagination {
                            page: server_page,
                            size: PAGE_SIZE,
                        }),
                        order: Some(Order {
                            direction: db_direction(spec.direction),
                            order_by: order_by_token(spec.field).to_string(),
                        }),
                        includes: Some(vec![PlaylistInclude::EpisodeIds]),
                        filter: filter_params,
                    };
                    client.list_playlists(params).await.map(|resp| resp.data)
                }
            },
        ),
    );
    let PagedPool {
        visible,
        loading,
        initial_settled,
        error,
        ..
    } = pool;

    // Remember scroll position + render window across navigation (in-memory, this
    // session). Restores the window so prior rows repaint from the cached pool,
    // then the offset; the stamp invalidates the offset across sort/search changes.
    use_paged_scroll_memory("playlists", "playlist-scroll", pool, sort, filter);

    // Sort/search change: reset the window + server cursor so the next fetch starts
    // a fresh (re-ordered / filtered) page sequence.
    let on_sort_change = use_callback(move |new_sort: SortSpec| {
        sort.set(new_sort);
        pool.reset();
    });
    let on_filter_change = use_callback(move |new_filter: FilterSpec| {
        filter.set(new_filter);
        pool.reset();
    });
    let on_new = use_callback(move |_| {
        nav.push("/playlists/create");
    });

    // Render the pool, subscribing to EpisodeState so a `CachePlaylists` upsert
    // re-renders. Name search + sort run client-side over the cached pool for
    // instant feedback; the server fetch fills in more matches as you scroll.
    let spec = sort.read().clone();
    let q = filter
        .read()
        .search
        .clone()
        .unwrap_or_default()
        .to_lowercase();
    let searching = !q.is_empty();
    // Allow reordering only for unfiltered Custom-ascending rows forming a contiguous position prefix. Otherwise
    // display indices can differ from server positions, moving a playlist to an unseen slot.
    let sort_allows_reorder =
        spec.field == SortField::Custom && spec.direction == OrderDirection::Asc && !searching;

    let state = playlists.read();
    let is_offline = use_connection().read().is_offline();
    let pool: Vec<PlaylistData> = state.playlists.clone();
    // Search + sort over the pool via the shared pure helper, then window it.
    let mut sorted = apply_playlist_search_sort(pool, &spec, &filter.read());
    sorted.truncate(visible());
    // Episode count per row: the playlist's own `episode_ids` if loaded, else the
    // separately-cached id list (`episodes_by_playlist`), else 0.
    let rows: Vec<(PlaylistData, usize)> = sorted
        .into_iter()
        .map(|pl| {
            let count = pl
                .episode_ids
                .as_ref()
                .map(|v| v.len())
                .or_else(|| state.episodes_by_playlist.get(&pl.id).map(|v| v.len()))
                .unwrap_or(0);
            (pl, count)
        })
        .collect();
    drop(state);

    // The drop index is only a valid server position when every VISIBLE row's
    // `position` equals its display index — i.e. the loaded/windowed pool is a
    // contiguous prefix from 0. Out-of-order-cached rows (see the note above)
    // break that, so disable the grip rather than send a wrong position.
    let positions_are_contiguous = rows
        .iter()
        .enumerate()
        .all(|(idx, (pl, _))| pl.position == idx as i32);
    let reorderable = sort_allows_reorder && positions_are_contiguous;

    let empty_message = if searching {
        "No playlists match your search."
    } else {
        "No playlists yet."
    };

    rsx! {
        // The shared `PagedListPage` owns the frame, scroll container, error/empty
        // boilerplate and the bottom sentinel; this page keeps the data wiring,
        // sort/filter reset, drag-reorder, and the per-row card markup. No
        // pull-to-refresh here (`pull: None`).
        PagedListPage {
            id_prefix: "playlist",
            error,
            loading,
            initial_settled,
            is_empty: rows.is_empty(),
            error_label: "playlists",
            empty_message: empty_message.to_string(),
            controls: rsx! {
                PlaylistListControls {
                    sort,
                    filter,
                    on_sort_change,
                    on_filter_change,
                    is_offline,
                    on_new,
                }
            },
            for (idx, (pl, count)) in rows.into_iter().enumerate() {
                {
                                let pid = pl.id;
                                let pl_name = pl.name.clone();
                                // Grip drag-to-reorder via Pointer Events — see
                                // `start_drag_reorder`. The reported target is the original
                                // `data-pl-index` of the card under the pointer, which
                                // `move_playlist` expects.
                                let on_grip_down = move |e: PointerEvent| {
                                    e.stop_propagation();
                                    if !reorderable {
                                        return;
                                    }
                                    start_drag_reorder(
                                        "playlist-scroll",
                                        "data-pl-index",
                                        &format!("pl-row-{pid}"),
                                        idx,
                                        e.client_coordinates().y,
                                        move |to| commands::move_playlist(&dispatch, pid, to as i32),
                                    );
                                };
                                rsx! {
                                    div {
                                        key: "{pid}",
                                        id: "pl-row-{pid}",
                                        "data-pl-index": "{idx}",
                                        class: "card bg-base-100 shadow-sm border border-base-300 hover:bg-base-200 transition-colors",
                                        div { class: "card-body p-3",
                                            div { class: "flex items-center justify-between gap-3",
                                                if reorderable {
                                                    // Drag handle — Pointer Events (mouse + touch);
                                                    // `touch-none` keeps the browser from scrolling
                                                    // while dragging from here. Sits outside the link
                                                    // so a grip drag never navigates.
                                                    span {
                                                        class: "shrink-0 cursor-grab active:cursor-grabbing text-muted touch-none",
                                                        "aria-label": "Drag to reorder",
                                                        onpointerdown: on_grip_down,
                                                        GripVertical { class: "w-5 h-5" }
                                                    }
                                                }
                                                // The card body is a real anchor to the detail
                                                // (keyboard-navigable, SPA nav); the grip beside it (a
                                                // sibling, not a child) owns the reorder gesture.
                                                Link {
                                                    to: format!("/playlists/{id}", id = pid),
                                                    class: "min-w-0 flex-1 flex items-center justify-between gap-3 cursor-pointer",
                                                    div { class: "min-w-0",
                                                        h2 { class: "text-sm font-semibold leading-snug truncate",
                                                            "{pl.name}"
                                                            if pl.is_default {
                                                                span { class: "badge badge-primary badge-sm ml-2", "Default" }
                                                            }
                                                        }
                                                        if let Some(desc) = pl.description.as_ref() {
                                                            p { class: "text-xs text-muted truncate mt-0.5", "{desc}" }
                                                        }
                                                    }
                                                    span { class: "badge badge-ghost badge-sm whitespace-nowrap shrink-0",
                                                        "{count} episodes"
                                                    }
                                                }
                                                // Kebab, playlist actions (reorder/configure/ delete; same builder as
                                                // the detail-header kebab). Sibling of the link so opening the menu
                                                // never navigates. Delete opens this page's confirm.
                                                KebabButton {
                                                    extra_class: "shrink-0",
                                                    label: "Playlist actions",
                                                    onclick: move |e: MouseEvent| {
                                                        e.stop_propagation();
                                                        quick.open(
                                                            pl_name.clone(),
                                                            playlist_menu_sections(
                                                                pid,
                                                                nav,
                                                                confirm.delete_playlist_callback(pid),
                                                            ),
                                                        );
                                                    },
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
        }
    }
}
