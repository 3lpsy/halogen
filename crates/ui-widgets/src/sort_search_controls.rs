//! Shared list-header controls: a sort dropdown (click a field to select it, click
//! again to flip direction) + a reveal-on-demand search bar, with a caller-supplied
//! action slot (`children`) between them. Used by the Podcasts and Playlists list
//! headers, which each supply their own sort fields + action button.
//!
//! The sort dropdown and the search bar themselves live in `controls_shared`
//! (shared with the episode-list `ListControls`, in `ui-episode-list`);
//! this component just lays them out with the caller's action between them.

use dioxus::prelude::*;

use crate::{FilterSpec, SearchInput, SearchToggle, SortDropdown, SortField, SortSpec};

#[component]
pub fn SortSearchControls(
    sort: Signal<SortSpec>,
    filter: Signal<FilterSpec>,
    on_sort_change: Callback<SortSpec>,
    on_filter_change: Callback<FilterSpec>,
    /// The sort fields offered, each with its menu label.
    fields: Vec<(SortField, &'static str)>,
    /// aria-label for the revealed search input (e.g. "Search podcasts").
    search_label: String,
    /// Placeholder for the revealed search input.
    search_placeholder: String,
    /// Action rendered between the sort dropdown and the search toggle (e.g. a
    /// "Discover" link or a "New playlist" button).
    children: Element,
) -> Element {
    // The search field is hidden behind an icon; tapping it reveals the bar below.
    let show_search = use_signal(|| false);

    rsx! {
        div { class: "flex flex-col",
            div { class: "flex items-center gap-2 p-2 bg-base-100 border-b border-base-200",
                // Sort dropdown — icon button (active field + direction in the list).
                SortDropdown { sort, on_sort_change, fields }

                // Spacer pushes the action + search controls to the right.
                div { class: "flex-1" }

                // Caller's action (Discover link / New playlist), left of search.
                {children}

                // Search toggle (far right) — reveals the search bar on the row below.
                SearchToggle { filter, on_filter_change, show_search }
            }

            // Revealed search bar.
            if show_search() {
                SearchInput { filter, on_filter_change, search_label, search_placeholder }
            }
        }
    }
}
