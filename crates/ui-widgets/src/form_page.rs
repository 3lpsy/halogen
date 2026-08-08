//! Shared chrome for the centered, scrollable form pages (create/edit forms and
//! similar): a full-height scroll container, a max-width centered column, and the
//! `BackButton` header. One place for the layout so a tweak can't drift across the
//! handful of form pages that share it.

use dioxus::prelude::*;

use crate::BackButton;

/// Wrap a form page's body in the standard scroll container + centered column +
/// back-button header. `scroll_id` sets the scroll element's DOM id when a page
/// needs it for scroll restoration (empty = no id).
#[component]
pub fn FormPage(#[props(default)] scroll_id: String, children: Element) -> Element {
    rsx! {
        div { id: "{scroll_id}", class: "h-full overflow-y-auto overflow-x-hidden",
            div { class: "p-2 max-w-lg mx-auto",
                div { class: "mb-2", BackButton {} }
                {children}
            }
        }
    }
}
