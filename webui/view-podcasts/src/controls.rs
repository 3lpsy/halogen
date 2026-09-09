//! Podcasts-list controls — a Title / Added sort dropdown + a search toggle, with
//! a "Discover" add action. A thin wrapper over the shared
//! [`SortSearchControls`](crate::components::SortSearchControls).

use dioxus::prelude::*;

use crate::components::{FilterSpec, SortField, SortSearchControls, SortSpec};
use halogen_webui_component_icons::Plus;

/// The only sort fields offered for podcasts (field + menu label).
const FIELDS: [(SortField, &str); 2] =
    [(SortField::Title, "Title"), (SortField::CreatedAt, "Added")];

#[component]
pub fn PodcastListControls(
    sort: Signal<SortSpec>,
    filter: Signal<FilterSpec>,
    on_sort_change: Callback<SortSpec>,
    on_filter_change: Callback<FilterSpec>,
) -> Element {
    rsx! {
        SortSearchControls {
            sort,
            filter,
            on_sort_change,
            on_filter_change,
            fields: FIELDS.to_vec(),
            search_label: "Search podcasts",
            search_placeholder: "Search...",
            // Add podcast — links to Discover.
            Link {
                to: "/discover",
                "aria-label": "Discover podcasts",
                class: "btn btn-square btn-ghost",
                Plus { class: "w-5 h-5" }
            }
        }
    }
}
