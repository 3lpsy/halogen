use dioxus::prelude::*;
use halogen_webui_component_widgets::{BackButton, ConfirmLinkModal, ExpandablePodcastDescription};
use halogen_webui_hooks::{use_config, use_discover_store};
use halogen_wire::{DiscoverPodcastParams, DiscoverResultItem};

use crate::{rows::EpisodeRow, subscribe::SubscribeButton};

/// Preserve old Discover detail links while using a separate remote podcast view.
#[component]
pub fn DiscoverDetail(id: String) -> Element {
    rsx! { DiscoverPodcast { id } }
}

#[component]
pub fn DiscoverPodcast(id: String) -> Element {
    let store = use_discover_store();
    let item = store.read().get(&id).cloned();
    rsx! {
        if let Some(item) = item { PodcastContent { key: "{id}", item } }
        else { MissingResult {} }
    }
}

#[component]
fn PodcastContent(item: DiscoverResultItem) -> Element {
    let config = use_config();
    let mut store = use_discover_store();
    let mut pending = use_signal(|| None::<String>);
    let original = item.clone();
    let mut preview = use_resource(move || {
        let item = original.clone();
        let cached = store
            .peek()
            .previews
            .iter()
            .find(|p| p.podcast.feed_url == item.feed_url)
            .cloned();
        let client = config.read().api_client();
        let generation = store.peek().generation;
        async move {
            if let Some(cached) = cached {
                return Ok(cached);
            }
            let client = client.ok_or_else(|| "No server configured".to_string())?;
            let data = client
                .discover_podcast(DiscoverPodcastParams {
                    feed_url: item.feed_url,
                    provider: item.provider,
                })
                .await
                .map_err(|e| e.to_string())?;
            if store.peek().generation == generation {
                let mut state = store.write();
                state
                    .previews
                    .retain(|p| p.podcast.feed_url != data.podcast.feed_url);
                if state.previews.len() >= 8 {
                    state.previews.remove(0);
                }
                state.previews.push(data.clone());
            }
            Ok::<_, String>(data)
        }
    });
    let loaded = preview
        .read()
        .as_ref()
        .and_then(|r| r.as_ref().ok())
        .cloned();
    let podcast = loaded.as_ref().map(|p| p.podcast.clone()).unwrap_or(item);
    rsx! {
        div { class: "h-full overflow-y-auto overflow-x-hidden",
            div { class: "p-2 space-y-4 max-w-4xl mx-auto",
                BackButton {}
                div { class: "flex items-start justify-between gap-3",
                    div { class: "min-w-0",
                        h1 { class: "text-2xl font-bold break-words", "{podcast.title}" }
                        if let Some(author) = &podcast.author { p { class: "text-sm text-muted", "{author}" } }
                        p { class: "text-xs text-muted mt-1", "{podcast.provider.label()} · Discover" }
                    }
                    SubscribeButton { podcast: podcast.clone(), disabled: preview.read().is_none() }
                }
                ExpandablePodcastDescription { html: podcast.description.clone(), on_link: move |url| pending.set(Some(url)) }
                h2 { class: "text-lg font-semibold", "Episodes" }
                if let Some(data) = loaded {
                    if data.episodes.is_empty() { p { class: "text-muted", "No episodes available in this feed." } }
                    if data.episodes.len() == 200 { p { class: "text-xs text-muted", "Showing up to 200 episodes from this feed." } }
                    for episode in data.episodes { EpisodeRow { key: "{episode.id}", item: episode } }
                } else {
                    match &*preview.read() {
                        Some(Err(error)) => rsx! {
                            div { class: "alert alert-error text-sm", role: "alert",
                                "Couldn't load this feed: {error}"
                                button { class: "btn btn-sm", onclick: move |_| preview.restart(), "Retry" }
                            }
                        },
                        _ => rsx! { div { role: "status", "aria-label": "Loading episodes", class: "py-4", span { class: "loading loading-spinner" } } },
                    }
                }
            }
            ConfirmLinkModal { pending }
        }
    }
}

#[component]
pub fn MissingResult() -> Element {
    rsx! {
        div { class: "p-2 space-y-4",
            BackButton {}
            p { "This Discover result is no longer available. Search again." }
            Link { to: "/discover", class: "btn btn-primary", "Back to Discover" }
        }
    }
}
