use dioxus::prelude::*;
use halogen_webui_component_widgets::{BackButton, ConfirmLinkModal, RichText};
use halogen_webui_hooks::use_discover_store;
use halogen_wire::{DiscoverEpisodeItem, DiscoverResultItem};

use crate::{detail::MissingResult, rows::episode_metadata, subscribe::SubscribeButton};

#[component]
pub fn DiscoverEpisode(id: String) -> Element {
    let store = use_discover_store();
    let episode = store.read().episode(&id).cloned();
    rsx! {
        if let Some(item) = episode { EpisodeContent { key: "{id}", item } }
        else { MissingResult {} }
    }
}

#[component]
fn EpisodeContent(item: DiscoverEpisodeItem) -> Element {
    let mut store = use_discover_store();
    let nav = use_navigator();
    let mut pending = use_signal(|| None::<String>);
    let podcast = store
        .read()
        .results
        .iter()
        .find(|p| p.feed_url == item.feed_url)
        .cloned()
        .or_else(|| {
            store
                .read()
                .previews
                .iter()
                .find(|p| p.podcast.feed_url == item.feed_url)
                .map(|p| p.podcast.clone())
        })
        .unwrap_or_else(|| DiscoverResultItem {
            id: format!("parent-{}", item.id),
            provider: item.provider,
            title: item.podcast_title.clone(),
            feed_url: item.feed_url.clone(),
            description: String::new(),
            author: None,
        });
    let parent = podcast.clone();
    let metadata = episode_metadata(&item);
    rsx! {
        div { class: "h-full overflow-y-auto overflow-x-hidden",
            div { class: "p-2 space-y-4 max-w-4xl mx-auto",
                BackButton {}
                button { class: "text-sm underline text-left break-words",
                    onclick: move |_| {
                        if store.peek().get(&parent.id).is_none() { store.write().results.push(parent.clone()); }
                        nav.push(format!("/discover/podcasts/{}", parent.id));
                    }, "{item.podcast_title}"
                }
                h1 { class: "text-2xl font-bold break-words", "{item.title}" }
                if !metadata.is_empty() { p { class: "text-sm text-muted", "{metadata}" } }
                SubscribeButton { podcast }
                if item.description.trim().is_empty() { p { class: "text-muted", "No episode description available." } }
                else { RichText { html: item.description, on_link: move |url| pending.set(Some(url)) } }
            }
            ConfirmLinkModal { pending }
        }
    }
}
