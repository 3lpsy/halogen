use dioxus::prelude::*;

use crate::Route;
use crate::components::{Artwork, Marquee};
use halogen_ui_state::hooks::{
    use_config, use_episodes, use_now_playing, use_play_context, use_playlists, use_podcasts,
};

/// "Up next" preview for the full-screen player: the episode the continuation
/// playlist (the play context, else the queue) will play when the current one
/// finishes ([`PlaylistState::next_up_in`]) — the same target as auto-advance
/// and the list's next-up marker, so they always agree.
///
/// Two tap targets: the artwork opens the next episode's detail page (and collapses
/// this full-screen player so the navigation is visible underneath); the text
/// column (label + title + podcast) force-scrolls the marquees, so a long title can
/// be read on demand even when `prefers-reduced-motion` (common on mobile / iOS Low
/// Power Mode) has frozen them. Renders nothing when the list has no next item.
#[component]
pub fn UpNext(on_close: Callback<()>) -> Element {
    let app_state = use_episodes();
    let podcasts = use_podcasts();
    let playlists = use_playlists();
    let now_playing = use_now_playing();
    let play_context = use_play_context();
    let config = use_config();
    let nav = use_navigator();
    // Tapping the text column opts into a scroll (overrides reduced-motion / short
    // text). Sticky for the life of this preview; a no-op if already scrolling.
    let mut force = use_signal(|| false);

    // Episode-granular memo: re-renders only when the *next* episode changes, not
    // on the player's ~4×/sec position writes to `now_playing`.
    let next_id = use_memo(move || {
        let current = now_playing.read().as_ref().map(|n| n.episode_id);
        playlists.read().next_up_in(current, play_context.read().0)
    });
    let Some(id) = next_id() else {
        return rsx! {};
    };
    // Tiny 9x9 thumbnail → use the downscaled `art_small`.
    let (title, podcast, _art, art_small) = app_state
        .read()
        .episode_display(&podcasts.read(), id, config.read().server_url.as_deref())
        .unwrap_or_else(|| ("Up next".to_string(), String::new(), None, None));

    rsx! {
        div { class: "flex items-center gap-2 w-full min-w-0",
            // Artwork → the next episode's detail page, collapsing the full-screen
            // player so the navigation is visible underneath.
            button {
                class: "flex-shrink-0 w-9 h-9 rounded bg-base-200 overflow-hidden",
                "aria-label": "Go to episode",
                onclick: move |_| {
                    nav.push(Route::EpisodeDetail { id });
                    on_close.call(());
                },
                Artwork {
                    src: art_small,
                    alt: "Up next artwork",
                    img_class: "w-full h-full object-cover",
                    placeholder_class: "text-xs",
                }
            }
            // Text column → force the labels to scroll (idempotent; a no-op if
            // they're already scrolling).
            button {
                class: "min-w-0 flex-1 text-left rounded-lg p-1 hover:bg-base-200",
                "aria-label": "Scroll up next: {title}",
                onclick: move |_| force.set(true),
                p { class: "text-[10px] uppercase tracking-wide text-muted leading-none mb-0.5",
                    "Up next"
                }
                Marquee { text: title.clone(), class: "text-xs font-medium", force: force() }
                Marquee { text: podcast, class: "text-[10px] text-muted", force: force() }
            }
        }
    }
}
