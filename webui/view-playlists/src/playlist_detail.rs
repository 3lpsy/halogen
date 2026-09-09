use dioxus::prelude::*;

use crate::components::{
    DetailHeaderBar, EpisodeList, FilterSpec, ItemVariant, KebabButton, ListSource, OrderDirection,
    SortField, SortSpec, playlist_menu_sections, use_confirm, use_quick_menu,
};
use halogen_webui_commands::actions as commands;
use halogen_webui_component_icons::Pencil;
use halogen_webui_hooks::{
    deep_link_placeholder, use_config, use_connection, use_deep_link_resource, use_dispatch,
    use_episodes, use_list_view_state, use_playlists,
};

/// Render playlist episodes through the shared offline-first lazy ID-list path. Key the body through a one-item list by
/// playlist ID so parameter-only navigation remounts deep-link guards and per-mount state.
#[component]
pub fn PlaylistDetail(id: i32) -> Element {
    rsx! {
        for id in [id] {
            PlaylistDetailBody { key: "{id}", id }
        }
    }
}

#[component]
fn PlaylistDetailBody(id: i32) -> Element {
    let app_state = use_episodes();
    let playlists = use_playlists();
    let config = use_config();
    let dispatch = use_dispatch();
    let nav = use_navigator();
    let quick = use_quick_menu();
    // Delete (in the kebab) opens the shared confirm; on confirm, navigate back
    // to the playlists list — this detail page's subject is gone (mirrors
    // `PodcastDetail`).
    let confirm = use_confirm();

    // Lazy / deep-link fetch: playlists aren't bulk-held, so fetch + cache this one
    // if it isn't in the pool. `load_failed` lets the empty state show "loading" vs
    // "not found". Mirrors `PodcastDetail`/`EpisodeDetail`.
    let load_failed = use_deep_link_resource(
        config,
        move || playlists.read().playlists.iter().any(|p| p.id == id),
        move |client| async move {
            let pl = client.get_playlist(id).await.map_err(|e| e.to_string())?;
            commands::cache_playlists(&dispatch, vec![pl]);
            Ok(())
        },
    );

    let playlist = playlists
        .read()
        .playlists
        .iter()
        .find(|p| p.id == id)
        .cloned();

    let is_offline = use_connection().read().is_offline();

    // Default to Custom (manual position order); the user can switch to any other
    // field via the controls. Remembered across visits (shared across playlists).
    let (sort, filter) = use_list_view_state(
        "playlist",
        SortSpec {
            field: SortField::Custom,
            direction: OrderDirection::Asc,
        },
        FilterSpec::default(),
    );

    rsx! {
        div { class: "flex flex-col h-full",
            // Back-button row with the edit (pencil) action on the right — mirrors the
            // podcast-detail header so the title block can stay compact and aligned
            // with the controls bar below. Pencil shows only when the playlist loaded.
            DetailHeaderBar {
                if let Some(pl) = playlist.as_ref() {
                    div { class: "flex items-center gap-1",
                        button {
                            "aria-label": "Edit playlist",
                            class: "flex items-center justify-center w-9 h-9 rounded text-muted hover:text-base-content hover:bg-base-200",
                            onclick: move |_| { nav.push(format!("/playlists/{id}/edit", id = id)); },
                            Pencil { class: "w-5 h-5" }
                        }
                        KebabButton {
                            label: "Playlist actions",
                            onclick: {
                                let name = pl.name.clone();
                                move |_| quick.open(
                                    name.clone(),
                                    playlist_menu_sections(
                                        id,
                                        nav,
                                        confirm.delete_playlist_then_callback(id, move |_| {
                                            nav.replace("/playlists");
                                        }),
                                    ),
                                )
                            },
                        }
                    }
                }
            }
            if let Some(pl) = playlist.as_ref() {
                // `p-2` aligns the name with the back button (px-2) and the controls
                // bar below; the smaller `text-lg` keeps the header from dominating.
                div { class: "p-2 border-b border-base-200",
                    h1 { class: "text-lg font-bold truncate",
                        "{pl.name}"
                        if pl.is_default {
                            span { class: "badge badge-primary badge-sm ml-2", "Default" }
                        }
                    }
                    if let Some(desc) = pl.description.as_ref() {
                        p { class: "text-sm text-muted mt-1 line-clamp-2", "{desc}" }
                    }
                }
            } else {
                div { class: "p-2",
                    p { class: "text-muted",
                        {deep_link_placeholder(load_failed(), is_offline, "playlist")}
                    }
                }
            }

            div { class: "flex-1 min-h-0",
                EpisodeList {
                    source: ListSource::Playlist { id },
                    sort,
                    filter,
                    // `RemoveFromList` targets *this* playlist via the item's
                    // `playlist_id` (derived from the `Playlist { id }` source).
                    swipe: config().swipe_prefs.playlist,
                    item_variant: ItemVariant::WithProgress,
                    app_state,
                    paged: true,
                    // Rows memory only (source-stamped) — no cross-playlist scroll.
                    rows_key: Some("playlist-detail"),
                }
            }
        }
    }
}
