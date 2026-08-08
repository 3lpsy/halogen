//! Shared lazy playlist multiselect, used by the two "pick playlists" screens
//! (an episode's playlists, a podcast's auto-add playlists).
//!
//! Renders the list body: a window over the cached playlist pool
//! (`EpisodeState.playlists`), revalidated page-by-page from the server in `position`
//! order with the search term driving server-side fetches, as a checkbox list.
//! The window always keeps currently-selected playlists visible (a member may sort
//! past the window) and, when searching, shows every loaded match.
//!
//! The caller owns everything around it — the search input (so it can be sticky or
//! in-form), the selection seeding, and the submit semantics — and passes `selected`
//! (toggled here) + the `search` signal. `on_change` fires after each toggle so a
//! form can clear stale errors. The host must render the scroll container with
//! `scroll_id` (the observer's root); this renders the matching `sentinel_id`.

use std::collections::HashSet;

use dioxus::prelude::*;
use halogen_wire::{
    DefaultListParams, FilterParams, Order, OrderDirection, Pagination, PlaylistData,
    PlaylistInclude,
};

use crate::Route;
use crate::components::BackButton;
use halogen_ui_config::api_client_from;
use halogen_ui_icons::MagnifyingGlass;
use halogen_ui_state::commands;
use halogen_ui_state::hooks::{
    FetchResult, PAGE_SIZE, PagedPool, Revalidate, use_config, use_connection, use_dispatch,
    use_paged_pool, use_playlists,
};

/// Lazy playlist multiselect list (see module docs).
#[component]
pub fn PlaylistMultiselect(
    scroll_id: String,
    sentinel_id: String,
    mut selected: Signal<HashSet<i32>>,
    search: Signal<String>,
    #[props(default)] on_change: Option<Callback<()>>,
) -> Element {
    let playlists = use_playlists();
    let dispatch = use_dispatch();
    let config = use_config();
    let nav = use_navigator();

    // Subscribe the revalidate effect only to the auth-relevant config slice (see
    // `paged_list_plan`) so an unrelated config edit while the picker is open doesn't
    // re-fetch the playlist pool.
    let auth = use_memo(move || {
        let c = config.read();
        (
            c.manual_offline,
            c.server_url.clone(),
            c.access_token.clone(),
        )
    });
    // Position-ordered, name-searched playlist pool (server-side order + filter, so
    // typing lazily pulls matching playlists rather than only filtering the pool).
    let pool = use_paged_pool(
        &scroll_id,
        &sentinel_id,
        move || playlists.peek().playlists.len(),
        move |server_page| {
            let q = search.read().trim().to_string();
            let (manual_offline, server_url, token) = auth();
            if manual_offline {
                // "Go Offline": serve the cached playlist pool only — no server fetch.
                return Revalidate::Skip;
            }
            let Some(client) = api_client_from(server_url.as_deref(), token.as_deref()) else {
                return Revalidate::Skip;
            };
            Revalidate::Fetch(Box::pin(async move {
                let filter = (!q.is_empty()).then(|| FilterParams {
                    search: Some(q.clone()),
                    ..Default::default()
                });
                let params = DefaultListParams::<PlaylistInclude> {
                    pagination: Some(Pagination {
                        page: server_page,
                        size: PAGE_SIZE,
                    }),
                    order: Some(Order {
                        direction: OrderDirection::Asc,
                        order_by: "position".to_string(),
                    }),
                    includes: Some(vec![PlaylistInclude::EpisodeIds]),
                    filter,
                };
                match client.list_playlists(params).await {
                    Ok(resp) => {
                        let has_more = resp.data.len() as i32 == PAGE_SIZE;
                        commands::cache_playlists(&dispatch, resp.data);
                        FetchResult {
                            has_more,
                            error: None,
                        }
                    }
                    Err(e) => FetchResult {
                        has_more: false,
                        error: playlists.peek().playlists.is_empty().then(|| e.to_string()),
                    },
                }
            }))
        },
    );
    let PagedPool {
        visible,
        loading,
        error,
        ..
    } = pool;

    // A new search term starts a fresh (filtered) page sequence.
    use_effect(move || {
        let _ = search();
        pool.reset();
    });

    // Rows: position-ordered window over the pool, client name-filtered for instant
    // feedback, always keeping selected playlists visible. When searching, show
    // every loaded match (no window).
    let q = search.read().trim().to_lowercase();
    let searching = !q.is_empty();
    let mut rows: Vec<PlaylistData> = playlists
        .read()
        .playlists
        .iter()
        .filter(|p| q.is_empty() || p.name.to_lowercase().contains(&q))
        .cloned()
        .collect();
    rows.sort_by(|a, b| a.position.cmp(&b.position).then(a.id.cmp(&b.id)));
    if !searching {
        let vis = visible();
        let sel = selected.read();
        rows = rows
            .into_iter()
            .enumerate()
            .filter(|(i, p)| *i < vis || sel.contains(&p.id))
            .map(|(_, p)| p)
            .collect();
    }

    let has_playlists = !playlists.read().playlists.is_empty();
    let err = error();
    let show_empty = rows.is_empty() && !loading() && err.is_none();

    rsx! {
        if let Some(msg) = err {
            div { class: "p-4 text-center text-error", "Error loading playlists: {msg}" }
        }
        if !has_playlists && show_empty {
            div { class: "text-center py-10",
                p { class: "text-muted mb-3", "You don't have any playlists yet." }
                button {
                    // Explicit type so this never submits a host `<form>` (the
                    // auto-playlists picker wraps us in one).
                    r#type: "button",
                    class: "btn btn-primary btn-sm",
                    onclick: move |_| {
                        nav.push(Route::PlaylistCreate {});
                    },
                    "Create a playlist"
                }
            }
        } else if show_empty {
            p { class: "p-8 text-center text-muted", "No playlists match your search." }
        } else {
            ul { class: "menu bg-base-100 rounded-box w-full p-2 border border-base-300",
                for pl in rows {
                    li { key: "{pl.id}",
                        label { class: "cursor-pointer flex items-center gap-3 py-1",
                            input {
                                r#type: "checkbox",
                                class: "checkbox checkbox-primary",
                                checked: selected.read().contains(&pl.id),
                                onchange: move |e| {
                                    // Drive off the browser's checked value (the source
                                    // of truth), not current set membership — they can
                                    // diverge if `selected` mutated since render.
                                    let pid = pl.id;
                                    let mut s = selected.write();
                                    if e.checked() {
                                        s.insert(pid);
                                    } else {
                                        s.remove(&pid);
                                    }
                                    drop(s);
                                    if let Some(cb) = on_change {
                                        cb.call(());
                                    }
                                },
                            }
                            span { class: "label-text", "{pl.name}" }
                        }
                    }
                }
            }
            div { id: "{sentinel_id}", class: "h-4" }
        }
    }
}

/// Full-page chrome around [`PlaylistMultiselect`] shared by the two playlist
/// picker screens (one episode, and a bulk multiselect): a back + title + search
/// header, the scrollable multiselect body, and a sticky Save footer with an
/// offline hint. The caller owns `selected` (it seeds/diffs it) and the save
/// semantics (`save_label`, `save_disabled`, `on_save`); everything else is here.
#[component]
pub fn PlaylistPickerScaffold(
    title: String,
    scroll_id: String,
    sentinel_id: String,
    selected: Signal<HashSet<i32>>,
    save_label: String,
    save_disabled: bool,
    on_save: EventHandler<MouseEvent>,
) -> Element {
    let playlists = use_playlists();
    let mut search = use_signal(String::new);

    let is_offline = use_connection().read().is_offline();
    let has_playlists = !playlists.read().playlists.is_empty();

    rsx! {
        div { class: "flex flex-col h-full",
            // Header: back + title + search box.
            div { class: "p-3 bg-base-100 border-b border-base-200 flex flex-col gap-2",
                div { class: "flex items-center gap-2",
                    BackButton {}
                    h1 { class: "text-lg font-semibold", "{title}" }
                }
                if has_playlists {
                    label { class: "input input-bordered flex items-center gap-2",
                        MagnifyingGlass { class: "w-4 h-4 opacity-60" }
                        input {
                            r#type: "text",
                            class: "grow",
                            placeholder: "Search playlists…",
                            value: "{search}",
                            oninput: move |e| search.set(e.value()),
                        }
                    }
                }
            }

            div {
                id: "{scroll_id}",
                class: "flex-1 overflow-y-auto overscroll-y-contain p-3",
                PlaylistMultiselect {
                    scroll_id: scroll_id.clone(),
                    sentinel_id,
                    selected,
                    search,
                }
            }

            // Sticky footer: Save + offline hint.
            if has_playlists {
                div { class: "p-3 bg-base-100 border-t border-base-200 flex flex-col gap-2",
                    button {
                        class: "btn btn-primary w-full",
                        disabled: save_disabled,
                        onclick: move |e| on_save.call(e),
                        "{save_label}"
                    }
                    if is_offline {
                        p { class: "text-xs text-muted text-center",
                            "You're offline — changes will sync when you reconnect."
                        }
                    }
                }
            }
        }
    }
}
