//! Shared render frames for the offline-first collection list pages
//! (`PagedListPage`) and simple list pages (`ListPage`).

use dioxus::prelude::*;

use crate::components::{EpisodeList, FilterSpec, ItemVariant, ListSource, SortSpec, SwipeConfig};
use halogen_webui_hooks::{use_episodes, use_list_view_state};

/// Share collection-page framing, scroll container, error/empty states, and sentinel. Callers supply controls, rows,
/// loading/error/emptiness, optional pull UI, and all data/reorder behavior. id_prefix defines matching
/// `{prefix}-scroll` and `{prefix}-sentinel` IDs for their hooks.
#[component]
pub fn PagedListPage(
    id_prefix: &'static str,
    error: Signal<Option<String>>,
    loading: Signal<bool>,
    initial_settled: Signal<bool>,
    is_empty: bool,
    error_label: &'static str,
    empty_message: String,
    controls: Element,
    pull: Option<Element>,
    children: Element,
) -> Element {
    let scroll_id = format!("{id_prefix}-scroll");
    let sentinel_id = format!("{id_prefix}-sentinel");
    let has_error = error.read().is_some();
    let show_loading = is_empty && (!initial_settled() || loading()) && !has_error;
    let show_empty = is_empty && !show_loading && !has_error;

    rsx! {
        div { class: "flex flex-col h-full",
            {controls}

            // Positioning context for the pull-to-refresh overlay. The indicator (`position: absolute`) sits OUTSIDE
            // the scroll container and anchors to this `relative` wrapper, so a background sync/poll flipping it
            // visible overlays the first row instead of pushing the whole list down, that in-flow-anchored shove was a
            // top CLS source (mirrors the episode list). `min-h-0` lets the scroll child shrink so it scrolls.
            div { class: "relative flex-1 min-h-0",
                if let Some(pull) = pull {
                    {pull}
                }

                div {
                    id: "{scroll_id}",
                    class: "h-full overflow-y-auto overscroll-y-contain",

                    if let Some(msg) = error() {
                        div { class: "p-4 text-center text-error", "Error loading {error_label}: {msg}" }
                    }
                    if show_empty {
                        p { class: "p-8 text-center text-muted", "{empty_message}" }
                    } else {
                        div { class: "flex flex-col",
                            {children}
                            // Cold load: reserve ~a screen of height with skeleton rows so
                            // the list doesn't jump when the first page lands (CLS). Only
                            // while nothing is shown yet; an append-more keeps the rows put.
                            if show_loading {
                                for i in 0..8 {
                                    div { key: "skel-{i}", class: "p-2",
                                        div { class: "skeleton h-20 w-full rounded-lg" }
                                    }
                                }
                            }

                        }
                    }
                    // Keep the observer target mounted through empty and loading states.
                    div { id: "{sentinel_id}", class: "h-4" }
                }
            }
        }
    }
}

/// Share episode-page view/scroll state and list props; callers provide swipe preferences. Optional sanitize_filter
/// transforms restored values once after mount. Pass values rather than child-owned signals to parent callbacks to
/// preserve Dioxus scope lifetimes.
#[component]
pub fn ListPage(
    view_key: &'static str,
    source: ListSource,
    default_sort: SortSpec,
    swipe: SwipeConfig,
    sanitize_filter: Option<Callback<FilterSpec, FilterSpec>>,
) -> Element {
    let app_state = use_episodes();
    let (sort, filter) = use_list_view_state(view_key, default_sort, FilterSpec::default());
    // One-shot mount hook (e.g. Downloads' OnDevice-token cleanup). Guarded so it
    // can't re-fire on later re-renders; the signal write stays in THIS scope, and
    // only happens when the sanitizer actually changed something.
    let mut mounted = use_signal(|| false);
    use_effect(move || {
        if mounted() {
            return;
        }
        mounted.set(true);
        if let Some(cb) = sanitize_filter {
            let mut filter = filter;
            let current = filter.peek().clone();
            let fixed = cb.call(current.clone());
            if fixed != current {
                filter.set(fixed);
            }
        }
    });
    rsx! {
        EpisodeList {
            source,
            sort,
            filter,
            swipe,
            item_variant: ItemVariant::WithProgress,
            app_state,
            paged: true,
            scroll_key: Some(view_key),
        }
    }
}
