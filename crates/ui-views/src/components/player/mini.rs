use crate::components::Artwork;
use dioxus::prelude::*;
use halogen_ui_appstate::EpisodeState;
use halogen_ui_icons::{Pause, Play, XMark};
use halogen_ui_state::hooks::{use_config, use_now_playing, use_player_controller, use_podcasts};
use halogen_ui_svc_player::PlaybackState;

/// Compact bottom player bar. Tapping the metadata opens the full-screen view
/// via `on_expand`.
///
/// Prop-type note: `app_state` is a `Signal<EpisodeState>` (not `ReadSignal`), so
/// dioxus's derived props memoize does NOT take the signal-specialized path
/// (it only matches `ReadSignal`/`ReadOnlySignal`); the prop compares by
/// `Signal`'s pointer-eq `PartialEq`, which is stable across parent renders —
/// this prop can never mark the app_state signal dirty. The component still
/// subscribes normally via the `.read()` in its body.
#[component]
pub fn MiniPlayer(app_state: Signal<EpisodeState>, on_expand: Callback<()>) -> Element {
    let player_controller = use_player_controller();
    let now_playing = use_now_playing();
    let podcasts = use_podcasts();
    let config = use_config();
    let Some(np) = now_playing.read().clone() else {
        return rsx! {};
    };
    // Mini player is a small chrome tile → use the thumbnail (`art_small`); the
    // full image is reserved for the expanded player.
    let (title, podcast, _art, art_small) = app_state
        .read()
        .episode_display(
            &podcasts.read(),
            np.episode_id,
            config.read().server_url.as_deref(),
        )
        .unwrap_or_else(|| ("Playing".to_string(), String::new(), None, None));

    rsx! {
        div {
            // Desktop: sits to the right of the sidebar (`md:left-64`) at the very
            // bottom. Mobile: floats just above the dock, whose total height is
            // 4rem + the iOS home-indicator inset (see Dock) — a plain `bottom-16`
            // left this bar overlapping the dock by the inset in an installed PWA.
            id: "mini-player",
            class: "absolute bottom-[calc(4rem+env(safe-area-inset-bottom))] md:bottom-0 left-0 md:left-64 right-0 bg-surface-1 border-t border-border shadow-lg z-40",
            style: "height: 4rem;",
            div { class: "flex items-center px-3 h-full",
                // Close: stop playback and dismiss the bar (stop() clears now_playing).
                button {
                    "aria-label": "Close player",
                    class: "w-8 h-8 flex items-center justify-center mr-1 flex-shrink-0 text-muted hover:text-base-content",
                    onclick: move |_| player_controller().stop(),
                    XMark { class: "w-5 h-5" }
                }
                // Artwork + title open the full-screen player.
                button {
                    class: "flex items-center flex-1 min-w-0 mr-2 text-left",
                    onclick: move |_| on_expand.call(()),
                    div { class: "w-12 h-12 rounded-md overflow-hidden mr-4 flex-shrink-0 bg-base-100",
                        // Optimistic server art; frown placeholder on miss.
                        Artwork {
                            src: art_small,
                            alt: "Episode artwork",
                            img_class: "w-full h-full object-cover",
                        }
                    }
                    div { class: "flex-1 min-w-0",
                        // Not a heading: this title sits in the global player chrome
                        // with no h1/h2 above it, so an <h3> here is an orphan that
                        // trips Lighthouse's heading-order. Styled identically.
                        div { class: "text-sm font-medium truncate", "{title}" }
                        p { class: "text-xs text-muted truncate", "{podcast}" }
                        // Mobile: progress stacks under the podcast name, full width.
                        // Desktop uses the side bar below instead.
                        if np.duration_secs.is_some() {
                            div { class: "md:hidden mt-1 h-1 bg-base-content/20 rounded-full overflow-hidden",
                                div { class: "h-full bg-primary", style: "width: {np.progress_pct()}%" }
                            }
                        }
                    }
                }
                if np.duration_secs.is_some() {
                    div { class: "hidden md:block w-24 h-1 bg-base-content/20 rounded-full overflow-hidden",
                        div { class: "h-full bg-primary", style: "width: {np.progress_pct()}%" }
                    }
                }
                button {
                    class: "w-10 h-10 flex items-center justify-center ml-2 text-base-content",
                    // Stable hook for e2e ("is it actually playing?") — the icons
                    // are bare SVGs with nothing else to select on.
                    "aria-label": if np.state == PlaybackState::Playing { "Pause" } else { "Play" },
                    onclick: move |_| player_controller().toggle(),
                    if np.state == PlaybackState::Playing {
                        Pause { class: "w-6 h-6" }
                    } else {
                        Play { class: "w-6 h-6" }
                    }
                }
            }
        }
    }
}
