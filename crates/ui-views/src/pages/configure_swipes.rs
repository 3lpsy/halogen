//! Customize-swipes screen (`/settings/configure-swipes`).
//!
//! Frontend-only editor for `ClientConfig.swipe_prefs`: per list page, pick the
//! action bound to each swipe direction. Persisted live to localStorage; the list
//! pages read the config and feed it straight into `EpisodeList` (each row resolves
//! and performs the action itself — see `EpisodeListItem`).

use dioxus::prelude::*;

use crate::components::{BackButton, SwipeAction};
use halogen_ui_config::SwipePage;
use halogen_ui_config::config_actions::persist_config;
use halogen_ui_state::hooks::use_config;

#[component]
pub fn ConfigureSwipes() -> Element {
    rsx! {
        div { class: "flex flex-col h-full overflow-hidden",
            // Pinned back-button row.
            div { class: "p-2", BackButton {} }
            // Scrollable list of per-page swipe pickers.
            div { class: "flex-1 overflow-y-auto overflow-x-hidden px-2 space-y-4",
                h1 { class: "text-2xl font-bold", "Customize Swipes" }
                p { class: "text-base-content/60 text-sm",
                    "Pick the action for each swipe direction, per page. Swiping a row to the right runs its \u{201C}Swipe right\u{201D} action; swiping left runs \u{201C}Swipe left\u{201D}."
                }
                for page in SwipePage::ALL {
                    SwipePageRow { key: "{page.label()}", page }
                }
            }
        }
    }
}

/// One page's section: its name plus a picker for each swipe side.
#[component]
fn SwipePageRow(page: SwipePage) -> Element {
    let config = use_config();
    let cfg = page.get(&config().swipe_prefs);
    rsx! {
        div { class: "space-y-2 p-3 bg-base-200 rounded-lg",
            h2 { class: "text-lg font-semibold", "{page.label()}" }
            SwipeSideSelect { page, left: true, current: cfg.left }
            SwipeSideSelect { page, left: false, current: cfg.right }
        }
    }
}

/// A single `<select>` bound to one swipe side of one page. Persists on change.
#[component]
fn SwipeSideSelect(page: SwipePage, left: bool, current: Option<SwipeAction>) -> Element {
    let config = use_config();
    let cur_token = current.map(|a| a.as_str()).unwrap_or("none");
    let side_label = if left { "Swipe right" } else { "Swipe left" };
    rsx! {
        label { class: "flex flex-wrap items-center justify-between gap-2",
            span { class: "text-muted", "{side_label}" }
            select {
                "aria-label": "{page.label()} {side_label}",
                class: "select select-bordered select-sm",
                value: "{cur_token}",
                onchange: move |e| {
                    let new = SwipeAction::from_token(&e.value());
                    persist_config(config, move |c| {
                        let mut cfg = page.get(&c.swipe_prefs);
                        if left {
                            cfg.left = new;
                        } else {
                            cfg.right = new;
                        }
                        page.set(&mut c.swipe_prefs, cfg);
                    });
                },
                option { value: "none", selected: current.is_none(), "None" }
                // Embedded mode drops the device-download actions from the
                // pickers (only the server-download concept exists there). A
                // stored device action keeps its option while selected — the
                // dispatch layer remaps it to the server counterpart anyway.
                for a in page.allowed_actions().into_iter().filter(|a| {
                    !config().server_kind.is_embedded()
                        || current == Some(*a)
                        || !matches!(
                            a,
                            SwipeAction::DownloadToDevice
                                | SwipeAction::RedownloadDevice
                                | SwipeAction::RemoveDownload
                                | SwipeAction::ToggleDownload
                        )
                }) {
                    option {
                        key: "{a.as_str()}",
                        value: "{a.as_str()}",
                        selected: current == Some(a),
                        "{a.label()}"
                    }
                }
            }
        }
    }
}
