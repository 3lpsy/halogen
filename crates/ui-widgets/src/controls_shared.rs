//! Shared list-header building blocks, used by both the episode-list controls
//! (`ListControls`, in `ui-episode-list`) and the podcasts/playlists
//! sort-search header ([`SortSearchControls`](super::sort_search_controls::SortSearchControls)).
//!
//! Both headers carried a byte-identical sort dropdown (click a field to select
//! it, click again to flip direction; Escape/Space keyboard handling) and the
//! same reveal-on-demand search bar (clear-X inside the field). Those two pieces
//! live here once so the markup/behavior can't drift between the two headers.

use dioxus::prelude::*;

use crate::{FilterSpec, OrderDirection, SortField, SortSpec};
use halogen_ui_icons::{ArrowsUpDown, MagnifyingGlass, XMark};

/// Keyboard handling for a daisyUI `role="button"` dropdown trigger. daisyUI opens
/// the menu on focus (so Tab already reveals it); this makes the `role="button"`
/// honour the keyboard too — Escape closes (blur, per daisyUI), Space doesn't
/// scroll the page. Shared by every icon-button dropdown in the list headers.
pub fn dropdown_keydown(e: KeyboardEvent) {
    let k = e.key().to_string();
    if k == "Escape" {
        let _ = document::eval("document.activeElement && document.activeElement.blur();");
    } else if k == " " {
        e.prevent_default();
    }
}

/// The sort dropdown: an icon button whose menu lists each `(field, label)`; the
/// active field shows its direction arrow. Clicking a field flips direction if it's
/// already selected, else selects it (keeping the current direction).
#[component]
pub fn SortDropdown(
    sort: Signal<SortSpec>,
    on_sort_change: Callback<SortSpec>,
    /// The sort fields offered, each with its menu label.
    fields: Vec<(SortField, &'static str)>,
) -> Element {
    let sort_val = sort.read().clone();
    let current_field = sort_val.field;
    let current_dir = sort_val.direction;

    // Click a field: flip direction if it's already selected, else select it
    // (keeping the current direction). `Callback` so it stays `Copy` for each row.
    let on_field_click = Callback::new(move |field: SortField| {
        let current = sort.read().clone();
        let direction = if current.field == field {
            match current.direction {
                OrderDirection::Asc => OrderDirection::Desc,
                OrderDirection::Desc => OrderDirection::Asc,
            }
        } else {
            current.direction
        };
        on_sort_change.call(SortSpec { field, direction });
    });

    // The active field's direction arrow, shown beside that field in the dropdown.
    let direction_arrow = if current_dir == OrderDirection::Asc {
        "↑"
    } else {
        "↓"
    };

    rsx! {
        div { class: "dropdown",
            div {
                tabindex: 0,
                role: "button",
                onkeydown: dropdown_keydown,
                class: "btn btn-square btn-ghost",
                "aria-label": "Sort",
                ArrowsUpDown { class: "w-5 h-5" }
            }
            ul {
                tabindex: 0,
                class: "dropdown-content menu bg-base-100 rounded-box w-44 p-2 shadow z-50",
                for (field, label) in fields.iter().copied() {
                    li {
                        button {
                            class: "flex items-center gap-2",
                            onclick: move |_| on_field_click.call(field),
                            span { "{label}" }
                            if field == current_field {
                                span { class: "text-xs ml-auto", "{direction_arrow}" }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// The search toggle button (rightmost in the header bar) — reveals the search
/// input ([`SearchInput`]) on the row below. Caller owns `show_search` so the
/// toggle can sit in the header's button row while the input renders below it.
///
/// Search isn't persisted (unlike sort/filter), so closing the bar reverts the
/// query rather than leaving a hidden active search.
#[component]
pub fn SearchToggle(
    filter: Signal<FilterSpec>,
    on_filter_change: Callback<FilterSpec>,
    /// `true` while the bar is revealed (caller-owned, paired with [`SearchInput`]).
    show_search: Signal<bool>,
) -> Element {
    let mut show_search = show_search;

    let on_search_change = move |value: String| {
        let mut new_filter = filter.read().clone();
        new_filter.search = if value.is_empty() { None } else { Some(value) };
        on_filter_change.call(new_filter);
    };

    rsx! {
        button {
            "aria-label": "Search",
            class: if show_search() { "btn btn-square btn-primary" } else { "btn btn-square btn-ghost" },
            onclick: move |_| {
                let opening = !show_search();
                show_search.set(opening);
                if !opening && filter.read().search.is_some() {
                    on_search_change(String::new());
                }
            },
            MagnifyingGlass { class: "w-4 h-4" }
        }
    }
}

/// The revealed search input row (rendered on its own line beneath the header
/// bar). Paired with [`SearchToggle`]; render it only while the bar is revealed.
#[component]
pub fn SearchInput(
    filter: Signal<FilterSpec>,
    on_filter_change: Callback<FilterSpec>,
    search_label: String,
    search_placeholder: String,
) -> Element {
    let on_search_change = move |value: String| {
        let mut new_filter = filter.read().clone();
        new_filter.search = if value.is_empty() { None } else { Some(value) };
        on_filter_change.call(new_filter);
    };

    rsx! {
        div { class: "p-2 bg-base-100 border-b border-base-200",
            div { class: "relative",
                input {
                    "aria-label": "{search_label}",
                    type: "text",
                    placeholder: "{search_placeholder}",
                    autofocus: true,
                    class: "input input-bordered w-full pr-10",
                    value: filter.read().search.clone().unwrap_or_default(),
                    oninput: move |e| on_search_change(e.value()),
                }
                // Clear (X) inside the field — erases the query, keeps the bar open.
                if filter.read().search.as_deref().is_some_and(|s| !s.is_empty()) {
                    button {
                        "aria-label": "Clear search",
                        class: "absolute right-2 top-1/2 -translate-y-1/2 btn btn-ghost btn-xs btn-circle",
                        onclick: move |_| on_search_change(String::new()),
                        XMark { class: "w-4 h-4" }
                    }
                }
            }
        }
    }
}
