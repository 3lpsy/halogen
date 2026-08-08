//! Visual indicator for the pull-to-refresh gesture.
//!
//! Rendered as the first child inside a list's scroll container; the gesture
//! itself is wired by [`use_pull_to_refresh`](halogen_ui_state::hooks::use_pull_to_refresh),
//! which drives the [`PullPhase`] this reads. The navbar and the sort/filter bar
//! sit outside the scroll container, so they never move with the pull.

use dioxus::prelude::*;

use halogen_ui_state::hooks::{PULL_THRESHOLD, PullPhase};

#[component]
pub fn PullToRefreshIndicator(phase: Signal<PullPhase>, is_admin: bool) -> Element {
    let p = phase();
    if matches!(p, PullPhase::Idle) {
        return rsx! {};
    }

    // Reveal height tracks the drag while pulling; a fixed (taller, so the text is
    // unmistakable on a phone) bar once armed/busy.
    let height = match p {
        PullPhase::Pulling(off) => off.clamp(0.0, 140.0),
        _ => 64.0,
    };

    // `absolute inset-x-0 top-0` overlays the bar on the top of the scroll region
    // (its parent is the `relative` wrapper around `#episode-scroll`) instead of
    // taking an in-flow box. In flow, a background sync/poll flipping Idle→Polling
    // pushed every row down ~64px and back — a top CLS source. As an overlay the
    // list never reflows; the bar briefly covers the first row during a sync.
    rsx! {
        div {
            class: "absolute inset-x-0 top-0 z-20 flex flex-col items-center justify-center gap-0.5 overflow-hidden \
                     border-b border-base-300 bg-base-200 text-sm font-medium text-base-content shadow-sm",
            style: "height: {height}px;",
            {
                match p {
                    PullPhase::Pulling(off) => {
                        let armed = off >= PULL_THRESHOLD;
                        let label = if armed { "Release to refresh" } else { "Pull to refresh" };
                        let arrow = if armed { "↑" } else { "↓" };
                        rsx! {
                            span { "{arrow} {label}" }
                            // Tease the hold-to-sync affordance for admins as they
                            // approach the threshold, so it's discoverable.
                            if is_admin && armed {
                                span { class: "text-xs font-normal text-muted", "keep holding to sync the server" }
                            }
                        }
                    }
                    PullPhase::Armed(n) => rsx! {
                        span { class: "flex items-center gap-2",
                            span { class: "loading loading-spinner loading-sm" }
                            "Release to refresh"
                        }
                        if is_admin {
                            span { class: "text-xs font-normal text-muted",
                                "or hold {n}s to sync the server…"
                            }
                        }
                    },
                    PullPhase::Refreshing => rsx! {
                        span { class: "flex items-center gap-2",
                            span { class: "loading loading-spinner loading-sm" }
                            "Refreshing…"
                        }
                    },
                    PullPhase::Polling => rsx! {
                        span { class: "flex items-center gap-2",
                            span { class: "loading loading-spinner loading-sm" }
                            "Syncing with server…"
                        }
                        span { class: "text-xs font-normal text-muted", "checking feeds for new episodes" }
                    },
                    PullPhase::Idle => rsx! {},
                }
            }
        }
    }
}
