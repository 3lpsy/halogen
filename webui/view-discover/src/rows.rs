use dioxus::prelude::*;
use halogen_webui_component_widgets::html::to_plain;
use halogen_wire::{DiscoverEpisodeItem, DiscoverResultItem};

#[component]
pub fn PodcastRow(item: DiscoverResultItem) -> Element {
    let plain = to_plain(&item.description);
    rsx! {
        Link { to: format!("/discover/podcasts/{}", item.id), class: "block py-4 border-b border-base-200 hover:bg-base-200/50 min-w-0",
            div { class: "font-semibold break-words", "{item.title}" }
            if let Some(author) = item.author { p { class: "text-xs text-muted", "{author}" } }
            p { class: "text-sm text-muted line-clamp-3 break-words", "{plain}" }
            p { class: "text-xs text-muted mt-1", "{item.provider.label()}" }
        }
    }
}

#[component]
pub fn EpisodeRow(item: DiscoverEpisodeItem) -> Element {
    let plain = to_plain(&item.description);
    let metadata = episode_metadata(&item);
    rsx! {
        Link { to: format!("/discover/episodes/{}", item.id), class: "block py-4 border-b border-base-200 hover:bg-base-200/50 min-w-0",
            div { class: "font-semibold break-words", "{item.title}" }
            p { class: "text-sm text-muted", "{item.podcast_title}" }
            if !metadata.is_empty() { p { class: "text-xs text-muted", "{metadata}" } }
            p { class: "text-sm text-muted line-clamp-3 break-words", "{plain}" }
        }
    }
}

pub fn episode_metadata(item: &DiscoverEpisodeItem) -> String {
    let mut parts = Vec::new();
    if let Some(date) = item
        .published_at
        .as_ref()
        .and_then(|d| chrono::DateTime::parse_from_rfc3339(d).ok())
    {
        parts.push(date.format("%b %-d, %Y").to_string());
    }
    if let Some(seconds) = item.duration_seconds {
        parts.push(format!("{} min", seconds.div_ceil(60)));
    }
    parts.join(" · ")
}
