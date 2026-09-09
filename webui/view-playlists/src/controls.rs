//! Playlists-list controls — a Custom / Name / Created sort dropdown, a search
//! toggle, and a "New playlist" action. A thin wrapper over the shared
//! [`SortSearchControls`](crate::components::SortSearchControls); sibling of
//! `podcasts::controls`.

use dioxus::prelude::*;

use crate::components::{FilterSpec, SortField, SortSearchControls, SortSpec};
use halogen_webui_component_icons::Plus;

/// The sort fields offered for playlists (field + menu label). `Custom` is the
/// manual `position` order (the default); `Title` carries the playlist name.
const FIELDS: [(SortField, &str); 3] = [
    (SortField::Custom, "Custom"),
    (SortField::Title, "Name"),
    (SortField::CreatedAt, "Created"),
];

#[component]
pub fn PlaylistListControls(
    sort: Signal<SortSpec>,
    filter: Signal<FilterSpec>,
    on_sort_change: Callback<SortSpec>,
    on_filter_change: Callback<FilterSpec>,
    is_offline: bool,
    on_new: Callback<()>,
) -> Element {
    rsx! {
        SortSearchControls {
            sort,
            filter,
            on_sort_change,
            on_filter_change,
            fields: FIELDS.to_vec(),
            search_label: "Search playlists",
            search_placeholder: "Search playlists...",
            // New playlist — disabled offline (creating needs the server).
            button {
                "aria-label": "New playlist",
                class: "btn btn-square btn-ghost",
                disabled: is_offline,
                title: if is_offline { "Connect to create a playlist" } else { "New playlist" },
                onclick: move |_| on_new.call(()),
                Plus { class: "w-5 h-5" }
            }
        }
    }
}
