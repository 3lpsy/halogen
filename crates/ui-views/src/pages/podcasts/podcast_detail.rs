use dioxus::prelude::*;

use crate::Route;
use crate::components::{
    Artwork, DetailHeaderBar, EpisodeList, FilterSpec, ItemVariant, KebabButton, ListSource,
    OrderDirection, SortField, SortSpec, podcast_menu_sections, use_confirm, use_quick_menu,
};
use halogen_ui_appstate::media_url;
use halogen_ui_state::commands;
use halogen_ui_state::hooks::{
    deep_link_placeholder, use_config, use_connection, use_deep_link_resource, use_dispatch,
    use_episodes, use_list_view_state, use_podcasts,
};

/// Podcast detail page — header (art, author, description) + every episode for
/// the podcast. The list reuses `EpisodeList` scoped via `FilterSpec.podcast_id`.
#[component]
pub fn PodcastDetail(id: i32) -> Element {
    let app_state = use_episodes();
    let podcasts = use_podcasts();
    let dispatch = use_dispatch();
    let config = use_config();
    let nav = use_navigator();

    // Podcasts aren't bulk-held; fetch + cache this one on a deep-link miss.
    // `load_failed` lets the empty state show "loading" vs "not found", and the
    // hook re-runs on a later publish so an offline miss recovers on its own.
    let load_failed = use_deep_link_resource(
        config,
        move || podcasts.read().podcast(id).is_some(),
        move |client| async move {
            let p = client.get_podcast(id).await.map_err(|e| e.to_string())?;
            commands::cache_podcasts(&dispatch, vec![p]);
            Ok(())
        },
    );

    let podcast = podcasts.read().podcast(id).cloned();
    let is_offline = use_connection().read().is_offline();
    // Drives the header config button: edit when the podcast already has a config,
    // create otherwise. The FK is always present even before the config body loads.
    let config_id_opt = podcast.as_ref().and_then(|p| p.podcast_config_id);

    // Scope the shared list to this podcast's episodes. Sort/filter/search are
    // remembered across visits (shared across podcasts); `podcast_id` is re-applied
    // by the page each mount and merged onto the restored state.
    let (sort, filter) = use_list_view_state(
        "podcast",
        SortSpec {
            field: SortField::PublishedAt,
            direction: OrderDirection::Desc,
        },
        FilterSpec {
            podcast_id: Some(id),
            ..FilterSpec::default()
        },
    );

    // Header actions (edit/create polling config, configure auto-playlists, delete)
    // live in a shared kebab menu on the back-button row — see `podcast_menu`. The
    // same builder backs the podcast list-item kebab. Delete opens this page's
    // confirm modal.
    let quick = use_quick_menu();
    let confirm = use_confirm();
    let menu_sections = podcast_menu_sections(
        id,
        config_id_opt,
        nav,
        confirm.purge_podcast_callback(id),
        confirm.delete_podcast_then_callback(id, move |_| {
            nav.replace(Route::Podcasts {});
        }),
    );
    let podcast_title = podcast.as_ref().map(|p| p.title.clone());

    rsx! {
        div { class: "flex flex-col h-full",
            DetailHeaderBar {
                if let Some(t) = podcast_title.clone() {
                    KebabButton {
                        label: "Podcast actions",
                        onclick: move |_| quick.open(t.clone(), menu_sections.clone()),
                    }
                }
            }
            if let Some(p) = podcast {
                div { class: "p-2 flex gap-4 border-b border-base-200",
                    div { class: "flex-shrink-0 w-24 h-24 rounded-lg bg-base-200 overflow-hidden flex items-center justify-center",
                        // Server art cache (optimistic; placeholder on miss). Detail
                        // view wants the full image, with the list's cached thumbnail
                        // shown instantly underneath while it loads.
                        Artwork {
                            src: media_url::art_url_for_podcast(config.read().server_url.as_deref(), &p),
                            placeholder_src: media_url::art_url_for_podcast_small(config.read().server_url.as_deref(), &p),
                            alt: "Podcast art",
                            img_class: "w-full h-full object-cover",
                            placeholder_class: "text-2xl",
                        }
                    }
                    div { class: "min-w-0 flex-1",
                        h1 { class: "text-2xl font-bold truncate", "{p.title}" }
                        if let Some(author) = p.author.clone() {
                            p { class: "text-sm text-muted", "{author}" }
                        }
                        p { class: "text-xs text-muted mt-1 line-clamp-3", "{p.description}" }
                    }
                }
            } else {
                div { class: "p-6",
                    p { class: "text-muted",
                        {deep_link_placeholder(load_failed(), is_offline, "podcast")}
                    }
                }
            }

            div { class: "flex-1 min-h-0",
                EpisodeList {
                    source: ListSource::AllEpisodes,
                    sort,
                    filter,
                    swipe: config().swipe_prefs.podcast,
                    item_variant: ItemVariant::WithProgress,
                    app_state,
                    paged: true,
                    // Rows memory only (filter-stamped) — no cross-podcast scroll.
                    rows_key: Some("podcast-detail"),
                }
            }
        }
    }
}
