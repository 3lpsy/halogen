//! Discover detail page — full description for one search result, plus an
//! "Add podcast" action (with confirmation) that subscribes by feed URL.
//!
//! The result is read from the ephemeral [`DiscoverState`](halogen_ui_appstate::DiscoverState)
//! by its synthetic id. A hard refresh / shared deep-link finds an empty store
//! and shows a "search again" fallback — Discover is online-only and never
//! persists results.

use dioxus::prelude::*;

use crate::Route;
use crate::components::{BackButton, ConfirmLinkModal, RichText};
use halogen_ui_state::commands;
use halogen_ui_state::hooks::{use_discover_store, use_dispatch, use_toast};

#[component]
pub fn DiscoverDetail(id: String) -> Element {
    let store = use_discover_store();
    let dispatch = use_dispatch();
    let toast = use_toast();
    let nav = use_navigator();
    let mut pending_link = use_signal(|| Option::<String>::None);
    let mut confirm_add = use_signal(|| false);

    let Some(item) = store.read().get(&id).cloned() else {
        return rsx! {
            div { class: "p-2",
                BackButton {}
                div { class: "mt-8 text-center space-y-4",
                    p { class: "text-muted",
                        "This result is no longer available — search again." }
                    button {
                        class: "btn btn-primary",
                        onclick: move |_| { nav.push(Route::Discover {}); },
                        "Back to Discover"
                    }
                }
            }
        };
    };

    // Captured by the confirm button's handler.
    let feed_url = item.feed_url.clone();
    let title = item.title.clone();
    let description = (!item.description.is_empty()).then(|| item.description.clone());
    let author = item.author.clone();

    rsx! {
        div { class: "h-full overflow-y-auto overflow-x-hidden",
        div { class: "p-2",
            BackButton {}
            span { class: "badge badge-outline mt-2", "{item.provider.label()}" }
            h1 { class: "text-2xl font-bold break-words mt-2", "{item.title}" }
            if let Some(author) = item.author.clone() {
                p { class: "text-sm text-muted mt-1", "{author}" }
            }
            p { class: "text-xs text-muted mt-1 break-words", "{item.feed_url}" }

            button {
                class: "btn btn-primary mt-4",
                onclick: move |_| confirm_add.set(true),
                "Add podcast"
            }

            if !item.description.is_empty() {
                div { class: "mt-5 text-sm text-base-content/80 break-words overflow-hidden",
                    RichText {
                        html: item.description.clone(),
                        on_link: move |href| pending_link.set(Some(href)),
                    }
                }
            } else {
                p { class: "mt-5 text-sm text-muted",
                    "No description provided by {item.provider.label()}." }
            }
        }
        ConfirmLinkModal { pending: pending_link }

        // Add-podcast confirmation.
        if confirm_add() {
            div { class: "modal modal-open",
                div { class: "modal-box", role: "dialog", "aria-modal": "true", "aria-labelledby": "confirm-add-podcast-title",
                    div { id: "confirm-add-podcast-title", class: "font-bold text-lg", "Add this podcast?" }
                    p { class: "py-3 text-sm text-base-content/80",
                        "Subscribe to \"{item.title}\" and start fetching its episodes?" }
                    p { class: "text-xs text-muted break-words", "{item.feed_url}" }
                    div { class: "modal-action",
                        button {
                            class: "btn btn-ghost",
                            onclick: move |_| confirm_add.set(false),
                            "Cancel"
                        }
                        button {
                            class: "btn btn-primary",
                            onclick: move |_| {
                                commands::subscribe_discovered(
                                    &dispatch,
                                    feed_url.clone(),
                                    title.clone(),
                                    description.clone(),
                                    author.clone(),
                                );
                                confirm_add.set(false);
                                toast.success(format!("Subscribing to {title}…"));
                                nav.push(Route::Podcasts {});
                            },
                            "Add podcast"
                        }
                    }
                }
                div {
                    class: "modal-backdrop",
                    onclick: move |_| confirm_add.set(false),
                }
            }
        }
        }
    }
}
