//! The `⋯` actions button shared by the detail-page headers and the playlists
//! list row. One place for the (long) class string so a styling change can't
//! drift across the four call sites.

use dioxus::prelude::*;

/// A `⋯` kebab button. `label` is the `aria-label`; `extra_class` prepends extra
/// classes (e.g. `shrink-0` for a flex-row sibling). The `onclick` handler runs
/// the menu open — call `e.stop_propagation()` inside it when the button sits over
/// a stretched link.
#[component]
pub fn KebabButton(
    label: String,
    onclick: EventHandler<MouseEvent>,
    #[props(default)] extra_class: String,
) -> Element {
    rsx! {
        button {
            class: "{extra_class} flex items-center justify-center w-9 h-9 text-lg leading-none rounded text-muted hover:text-base-content hover:bg-base-200",
            "aria-label": "{label}",
            onclick: move |e| onclick.call(e),
            "⋯"
        }
    }
}
