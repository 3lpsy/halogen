use dioxus::prelude::*;

use halogen_webui_component_icons::{Funnel, Pencil, ViewfinderCircle, XMark};
use halogen_webui_component_widgets::{SearchInput, SearchToggle, SortDropdown, dropdown_keydown};
use halogen_webui_listview::{
    EpisodeFilter, FilterSpec, ListSource, MultiSelectState, SortField, SortSpec,
};

#[component]
pub fn ListControls(
    source: ListSource,
    sort: Signal<SortSpec>,
    filter: Signal<FilterSpec>,
    on_sort_change: Callback<SortSpec>,
    on_filter_change: Callback<FilterSpec>,
    /// Multiselect view state (shared with `EpisodeList`): drives the activate /
    /// count / exit controls and the "Selected" review chip.
    ms: Signal<MultiSelectState>,
    /// Open the bulk-action menu (built by `EpisodeList`, which has the dispatch +
    /// selection). Called when the count badge is tapped.
    on_open_menu: Callback<()>,
    /// Select every currently-loaded row (built by `EpisodeList`, which owns the
    /// loaded set). Called by the "Select all" button in the multiselect sub-bar.
    on_select_all: Callback<()>,
    /// Embedded-server mode: drops the OnDevice facet (no separate device set).
    #[props(default)]
    embedded: bool,
) -> Element {
    // Mutable copy of the multiselect signal (Copy) so the onclick closures below
    // can write it. `ms_active`/`ms_count`/`selected_only` are this-render snapshots.
    let mut ms = ms;
    let ms_active = ms.read().active;
    let ms_count = ms.read().selected.len();
    let selected_only = ms.read().selected_only;

    // The search field is hidden behind an icon; tapping it reveals the bar below.
    let show_search = use_signal(|| false);

    // The sort fields offered for this source, each with its dropdown label.
    let available_fields: Vec<(SortField, &'static str)> = match &source {
        // Custom (manual position order) is offered only for playlists.
        ListSource::Playlist { .. } => vec![
            (SortField::Custom, "Play Order"),
            (SortField::PublishedAt, "Published"),
            (SortField::Title, "Title"),
            (SortField::CreatedAt, "Added"),
            (SortField::Duration, "Duration"),
        ],
        ListSource::AllEpisodes | ListSource::History => vec![
            (SortField::PublishedAt, "Published"),
            (SortField::Title, "Title"),
            (SortField::CreatedAt, "Added"),
            (SortField::Duration, "Duration"),
        ],
        ListSource::ClientDownloads | ListSource::ServerDownloads => vec![
            (SortField::PublishedAt, "Published"),
            (SortField::CreatedAt, "Added"),
            (SortField::Duration, "Duration"),
        ],
    };

    // Multi-select filter chips — toggle membership in `filter.filters`.
    let toggle_filter = Callback::new(move |f: EpisodeFilter| {
        let mut new_filter = filter.read().clone();
        if let Some(pos) = new_filter.filters.iter().position(|x| *x == f) {
            new_filter.filters.remove(pos);
        } else {
            new_filter.filters.push(f);
        }
        on_filter_change.call(new_filter);
    });

    // Downloads already defines the device set, so omit OnDevice and server-download chips there. Keep played-state
    // chips in order: never started, in progress, finished; label Played as In Progress to avoid implying completion.
    let filter_options: Vec<(EpisodeFilter, &'static str)> = match &source {
        // Both downloads-page sources ARE the download set — a download facet
        // would be a no-op.
        ListSource::ClientDownloads | ListSource::ServerDownloads => vec![
            (EpisodeFilter::Unplayed, "Unplayed"),
            (EpisodeFilter::Played, "In Progress"),
            (EpisodeFilter::Finished, "Finished"),
        ],
        // Every other source offers the full facet set. On the cache-query lists (/latest, podcast detail) OnDevice
        // can't be expressed server-side, so picking it switches the list to the local device set (handled in
        // `EpisodeList`) rather than paging the server. Embedded mode drops the OnDevice facet, there is no separate
        // device set.
        _ => {
            let mut options = vec![
                (EpisodeFilter::OnDevice, "On Device"),
                (EpisodeFilter::Downloaded, "Downloaded"),
                (EpisodeFilter::Downloading, "Downloading"),
                (EpisodeFilter::Unplayed, "Unplayed"),
                (EpisodeFilter::Played, "In Progress"),
                (EpisodeFilter::Finished, "Finished"),
            ];
            if embedded {
                options.retain(|(f, _)| *f != EpisodeFilter::OnDevice);
            }
            options
        }
    };
    let active_filter_count = filter.read().filters.len();

    rsx!(
        div { class: "flex flex-col",
        div { class: "flex items-center gap-2 p-2 bg-base-100 border-b border-base-200",
            // Sort dropdown — icon button (active field + direction in the list).
            // Icon-only keeps the bar compact so it doesn't overflow at large UI
            // font sizes. Shared with the podcasts/playlists header (`SortDropdown`).
            SortDropdown { sort, on_sort_change, fields: available_fields }

            // Filter dropdown (download + played state) — sits next to Sort on the
            // left. Plain `dropdown` (not `dropdown-end`) so it opens left-aligned
            // under the button like Sort. Icon button with a count badge; matches
            // Sort/search for a compact, overflow-proof bar.
            div { class: "dropdown",
                div {
                    tabindex: 0,
                    role: "button",
                    onkeydown: dropdown_keydown,
                    class: "btn btn-square btn-ghost relative",
                    "aria-label": "Filter",
                    Funnel { class: "w-5 h-5" }
                    if active_filter_count > 0 {
                        span { class: "badge badge-primary badge-sm absolute -top-1 -right-1", "{active_filter_count}" }
                    }
                }
                ul {
                    tabindex: 0,
                    class: "dropdown-content menu bg-base-100 rounded-box w-44 p-2 shadow z-50",
                    for (variant, text) in filter_options {
                        li {
                            label { class: "cursor-pointer flex items-center gap-2",
                                input {
                                    type: "checkbox",
                                    class: "checkbox checkbox-primary",
                                    checked: filter.read().filters.contains(&variant),
                                    onchange: move |_| toggle_filter.call(variant),
                                }
                                span { "{text}" }
                            }
                        }
                    }
                }
            }

            // Spacer pushes the search + multiselect controls to the right.
            div { class: "flex-1" }

            // Multiselect toggle — left of search. A single button: tap to enter
            // multiselect, tap again (highlighted) to exit + clear. The selection
            // controls (Selected chip, action menu, exit) live in the dedicated
            // multiselect sub-bar below, keeping this core bar uncluttered.
            button {
                "aria-label": if ms_active { "Exit multiselect" } else { "Select multiple" },
                class: if ms_active { "btn btn-square btn-primary" } else { "btn btn-square btn-ghost" },
                onclick: move |_| {
                    let mut s = ms.write();
                    if s.active {
                        s.active = false;
                        s.selected_only = false;
                        s.selected.clear();
                    } else {
                        s.active = true;
                    }
                },
                ViewfinderCircle { class: "w-5 h-5" }
            }

            // Search toggle — always rightmost; reveals the search bar on the row
            // below. Shared with the podcasts/playlists header (`SearchToggle`).
            SearchToggle { filter, on_filter_change, show_search }
        }

        // Multiselect sub-bar, only while multiselect is active. Left: the "Selected" review chip (restrict the list to
        // the ticked set; selection survives toggling it off) and "Select all" (ticks every loaded row). Right: a
        // pencil that opens the bulk-action menu (count badge), then an exit (X) that leaves multiselect and clears.
        if ms_active {
            div { class: "flex items-center gap-2 p-2 bg-base-100 border-b border-base-200",
                button {
                    class: if selected_only { "btn btn-sm btn-primary gap-1" } else { "btn btn-sm btn-outline gap-1" },
                    onclick: move |_| {
                        let mut s = ms.write();
                        s.selected_only = !s.selected_only;
                    },
                    "Selected"
                }
                // Selects the LOADED rows only (what's been fetched/scrolled so
                // far) — rows loaded afterwards are not auto-selected.
                button {
                    class: "btn btn-sm btn-outline gap-1",
                    onclick: move |_| on_select_all.call(()),
                    "Select all"
                }
                div { class: "flex-1" }
                button {
                    "aria-label": "Bulk actions",
                    class: "btn btn-square btn-ghost relative",
                    onclick: move |_| on_open_menu.call(()),
                    Pencil { class: "w-5 h-5" }
                    if ms_count > 0 {
                        span { class: "badge badge-primary badge-sm absolute -top-1 -right-1", "{ms_count}" }
                    }
                }
                button {
                    "aria-label": "Exit multiselect",
                    class: "btn btn-square btn-ghost",
                    onclick: move |_| {
                        let mut s = ms.write();
                        s.active = false;
                        s.selected_only = false;
                        s.selected.clear();
                    },
                    XMark { class: "w-4 h-4" }
                }
            }
        }

        // Revealed search bar (shared `SearchInput`).
        if show_search() {
            SearchInput {
                filter,
                on_filter_change,
                search_label: "Search episodes",
                search_placeholder: "Search...",
            }
        }
        }
    )
}
