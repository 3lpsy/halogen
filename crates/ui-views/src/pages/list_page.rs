//! Shared render frames for the offline-first collection list pages
//! (`PagedListPage`) and simple list pages (`ListPage`).

use dioxus::prelude::*;

use crate::components::{EpisodeList, FilterSpec, ItemVariant, ListSource, SortSpec, SwipeConfig};
use halogen_ui_state::hooks::{use_episodes, use_list_view_state};

/// Shared render frame for the offline-first **collection** list pages (Podcasts,
/// Playlists) that render a window over a cached pool and revalidate it
/// page-by-page. It owns the bits both pages repeated verbatim: the
/// `flex flex-col h-full` frame, the scroll container, the error/empty boilerplate
/// and the bottom infinite-scroll sentinel.
///
/// What stays caller-side is what diverges between the two — the data wiring
/// (`use_paged_pool` / `use_paged_scroll_memory`, cache source, sort/filter reset),
/// pull-to-refresh enablement, drag-reorder, and the per-row card markup — so the
/// scaffold never has to know any page-specific behavior. The page passes:
/// - its `controls` element and the per-row `children` (the rows only — the
///   scaffold appends the sentinel),
/// - the `error`/`loading` signals it already destructured from its `PagedPool`,
///   plus whether the rendered set `is_empty` (the row types differ, so the page
///   computes emptiness),
/// - an optional `pull` indicator (Some for Podcasts, None for Playlists).
///
/// The `id_prefix` (`"podcast"`, `"playlist"`) single-sources the scroll and
/// sentinel dom ids — `{prefix}-scroll` and `{prefix}-sentinel` — that the page's
/// own hooks (`use_paged_pool`, `use_pull_to_refresh`, `start_drag_reorder`)
/// reference by the same names.
#[component]
pub fn PagedListPage(
    id_prefix: &'static str,
    error: Signal<Option<String>>,
    loading: Signal<bool>,
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
    let show_empty = is_empty && !loading() && !has_error;

    rsx! {
        div { class: "flex flex-col h-full",
            {controls}

            // Positioning context for the pull-to-refresh overlay. The indicator
            // (`position: absolute`) sits OUTSIDE the scroll container and anchors to
            // this `relative` wrapper, so a background sync/poll flipping it visible
            // overlays the first row instead of pushing the whole list down — that
            // in-flow-anchored shove was a top CLS source (mirrors the episode list).
            // `min-h-0` lets the scroll child shrink so it actually scrolls.
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
                            if loading() && is_empty {
                                for i in 0..8 {
                                    div { key: "skel-{i}", class: "p-2",
                                        div { class: "skeleton h-20 w-full rounded-lg" }
                                    }
                                }
                            }
                            // Sentinel: scrolling near it grows the window + pulls the next page.
                            div { id: "{sentinel_id}", class: "h-4" }
                        }
                    }
                }
            }
        }
    }
}

/// Shared scaffold for the paged episode-list pages (Latest, History, the Queue's
/// populated state, and Downloads): wires `use_list_view_state` (keyed by
/// `view_key`, which also seeds scroll memory) and the `EpisodeList` props that are
/// identical across them. Each row resolves and performs its own swipe (see
/// `EpisodeListItem`), so the page only supplies the per-page `swipe` config.
///
/// `sanitize_filter` is an optional one-shot hook run (via a guarded `use_effect`)
/// the first time the page renders: it receives the RESTORED `FilterSpec` by value
/// and returns the (possibly edited) value to commit — used by Downloads to
/// self-heal a stale persisted filter token. Deliberately value-in/value-out, NOT
/// the page's signals: those are owned by this scope, and handing them to a
/// parent-owned callback trips dioxus' copy-value-hoist warning (the parent scope
/// outlives them, so it could touch them after they're dropped). Defaulted to
/// `None`, so the other list pages are unaffected.
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
