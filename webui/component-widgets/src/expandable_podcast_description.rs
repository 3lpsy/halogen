use dioxus::prelude::*;

use crate::{RichText, html::to_plain};

/// Podcast summaries start as three plain-text lines; expansion preserves safe links and formatting.
#[component]
pub fn ExpandablePodcastDescription(html: String, on_link: Callback<String>) -> Element {
    let mut expanded = use_signal(|| false);
    let plain = to_plain(&html);
    rsx! {
        if !plain.trim().is_empty() {
            div { class: "text-sm break-words min-w-0",
                if expanded() {
                    RichText { html, on_link }
                } else {
                    p { class: "line-clamp-3 whitespace-pre-line", "{plain}" }
                }
                button {
                    r#type: "button", class: "btn btn-ghost btn-xs mt-1",
                    "aria-expanded": expanded().to_string(),
                    onclick: move |_| expanded.toggle(),
                    if expanded() { "Show less" } else { "Show more" }
                }
            }
        }
    }
}
