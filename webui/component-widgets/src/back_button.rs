use dioxus::prelude::*;

use halogen_webui_component_icons::ChevronLeft;

/// A history "back" button (`navigator.go_back()`). Used on detail pages (episode / podcast) so they return to wherever
/// the user came from, e.g. Episode-from-Queue goes back to Queue, Episode-from-Podcast goes back to the podcast. The
/// main nav pages (Queue, Latest, History, Playlists, Downloads) intentionally do NOT render it.
#[component]
pub fn BackButton() -> Element {
    let nav = use_navigator();
    rsx! {
        // `pl-0` (no left padding) so the chevron sits at the container's content
        // edge — flush with whatever renders below (art, list, form fields), which
        // share the back button's padding. Previously `-ml-2` nudged the whole
        // button left but its own left padding still pushed the glyph out of line.
        button {
            class: "btn btn-ghost gap-1 mb-2 pl-0",
            onclick: move |_| {
                nav.go_back();
            },
            ChevronLeft { class: "w-4 h-4" }
            "Back"
        }
    }
}

/// The detail-page top bar: a [`BackButton`] on the left and `children` (the
/// page's action controls — a kebab, an edit pencil, …) on the right. The
/// podcast / playlist / episode detail pages all opened with this exact row, so
/// the layout convention lives here once.
#[component]
pub fn DetailHeaderBar(children: Element) -> Element {
    rsx! {
        div { class: "px-2 pt-2 flex items-center justify-between",
            BackButton {}
            {children}
        }
    }
}
