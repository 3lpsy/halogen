use super::sleep_button::SleepTimerButton;
use crate::UpNext;
use crate::components::{Artwork, Marquee};
use dioxus::prelude::*;
use halogen_format::format_time;
use halogen_webui_app_state::EpisodeState;
use halogen_webui_commands::actions as commands;
use halogen_webui_component_icons::{
    BackwardStep, Bolt, ChevronDown, CircleA, CircleAOff, ForwardStep, ListUl, Pause, Play,
};
use halogen_webui_config::PLAYBACK_RATES;
use halogen_webui_config::config_actions::persist_config;
use halogen_webui_hooks::{
    use_config, use_dispatch, use_now_playing, use_play_context, use_player_controller,
    use_playlists, use_podcasts,
};
use halogen_webui_player::PlaybackState;
use halogen_wire::EpisodeChapterData;

/// Drag distance (px) past which releasing the full-screen player dismisses it.
const DRAG_DISMISS_PX: f64 = 120.0;

/// Pin controls below shrinking art/title in a non-scrolling column so large fonts cannot hide playback controls. Drag
/// the upper zone down to collapse. Signal props compare by pointer and never dirty episode state through prop
/// memoization.
#[component]
pub fn NowPlayingScreen(app_state: Signal<EpisodeState>, on_close: Callback<()>) -> Element {
    let player = use_player_controller();
    let now_playing = use_now_playing();
    let podcasts = use_podcasts();
    let playlists = use_playlists();
    let dispatch = use_dispatch();
    // Configured skip intervals (also used by the hardware media controls).
    let config = use_config();
    let skip_forward = config().playback_prefs.skip_forward;
    let skip_backward = config().playback_prefs.skip_backward;
    // Auto-advance to the next queue item — same setting as Settings, exposed
    // here as a quick toggle next to the speed control.
    let auto_advance = config().playback_prefs.auto_advance;

    // Lazily load the current episode's chapters while the expanded player is open.
    // The memo gates on the episode id (PartialEq), so the effect re-fires only on
    // a track change — not on every ~4×/sec position tick. The worker de-dups and
    // the merge is sticky, so a repeat call is a cheap no-op.
    let current_episode = use_memo(move || now_playing.read().as_ref().map(|n| n.episode_id));
    use_effect(move || {
        if let Some(id) = current_episode() {
            commands::ensure_episode_chapters(&dispatch, id);
        }
    });

    // Memoize adjacent-track scans on episode, playlist, and play-context changes, not position ticks. Playlist/context
    // subscriptions are required even when the current track is unchanged; read the stable controller with peek.
    let play_context = use_play_context();
    let has_prev = use_memo(move || {
        let _ = current_episode();
        let _ = app_state.read();
        let _ = playlists.read();
        let _ = play_context.read();
        player.peek().has_previous()
    });
    let has_next = use_memo(move || {
        let _ = current_episode();
        let _ = app_state.read();
        let _ = playlists.read();
        let _ = play_context.read();
        player.peek().has_next()
    });

    let Some(np) = now_playing.read().clone() else {
        return rsx! {};
    };
    // Full-screen art uses the original; `art_small` is its instant placeholder
    // (already cached from the list/mini player) while the original streams in.
    let (title, podcast, art, art_small) = app_state
        .read()
        .episode_display(
            &podcasts.read(),
            np.episode_id,
            config.read().server_url.as_deref(),
        )
        .unwrap_or_else(|| ("Now Playing".to_string(), String::new(), None, None));
    let duration = np.duration_secs.unwrap_or(0.0);

    // Chapters for the now-playing episode (empty until the lazy fetch lands, or
    // when the episode has none). `active` is the marker currently playing.
    let chapters = app_state
        .read()
        .episode(np.episode_id)
        .and_then(|e| e.chapters.clone())
        .unwrap_or_default();
    let active_chapter = active_chapter_index(&chapters, np.position_secs);
    let has_ticks = duration > 0.0 && !chapters.is_empty();

    // Keep drag position locally so playback ticks cannot overwrite the seek thumb. Render its tracked position while
    // scrubbing and seek from it on release, never from the possibly overwritten event value.
    let mut scrubbing = use_signal(|| false);
    let mut scrub_pos = use_signal(|| 0.0_f64);
    let shown_pos = if scrubbing() {
        scrub_pos()
    } else {
        np.position_secs
    };

    // Drag-down-to-dismiss. Pressing anywhere in the upper zone (header / art / title) and dragging down past the
    // threshold collapses to the mini player; a shorter tug snaps back. Move/up live on the root so the gesture keeps
    // tracking if the finger leaves the start element; the bottom controls are a sibling of the start zone, so pressing
    // them never begins a drag.
    let mut drag_start_y = use_signal(|| None::<f64>);
    let mut drag_offset = use_signal(|| 0.0_f64);

    let on_drag_start = move |e: PointerEvent| {
        drag_start_y.set(Some(e.client_coordinates().y));
    };
    let on_drag_move = move |e: PointerEvent| {
        if let Some(start) = drag_start_y() {
            // Downward only; an upward tug just holds at 0.
            drag_offset.set((e.client_coordinates().y - start).max(0.0));
        }
    };
    let on_drag_end = use_callback(move |_: PointerEvent| {
        if drag_start_y().is_some() {
            let off = drag_offset();
            drag_start_y.set(None);
            if off > DRAG_DISMISS_PX {
                // Leave the offset as-is — the parent unmounts us on close, so
                // resetting to 0 here would flash a snap-up first.
                on_close.call(());
            } else {
                drag_offset.set(0.0);
            }
        }
    });

    // Animate the snap-back, but never while actively dragging (the sheet must
    // track the finger 1:1).
    let snap = if drag_start_y().is_some() {
        ""
    } else {
        "transition-transform duration-200"
    };

    rsx! {
        // Full-bleed overlay; safe-area padding clears the notch + home indicator
        // (no-ops in a browser tab). z-[60] sits above the mobile dock (z-50).
        div {
            class: "absolute inset-0 bg-base-100 z-[60] flex flex-col pt-[env(safe-area-inset-top)] pb-[env(safe-area-inset-bottom)] {snap}",
            style: "transform: translateY({drag_offset}px);",
            onpointermove: on_drag_move,
            onpointerup: move |e| on_drag_end.call(e),
            onpointercancel: move |e| on_drag_end.call(e),

            // Header — collapse affordance (left) + the queue "Up next" preview
            // (right). Kept OUTSIDE the drag zone below so tapping Up Next (it
            // navigates) can never begin a pull-to-dismiss.
            div { class: "flex items-center gap-3 p-4 shrink-0",
                button {
                    "aria-label": "Collapse player",
                    class: "text-base-content",
                    onclick: move |_| on_close.call(()),
                    ChevronDown { class: "w-6 h-6" }
                }
                // Up next — queue continuation; text scrolls if it doesn't fit.
                // Renders nothing when the queue has no next item. `on_close` lets
                // its artwork tap collapse this player on navigation.
                div { class: "ml-auto w-1/2 max-w-xs flex justify-end",
                    UpNext { on_close }
                }
            }

            // Upper zone — the drag handle for pull-to-dismiss. `touch-none`
            // (touch-action: none) stops the mobile browser claiming the
            // downward pan for scroll/overscroll, which would fire
            // `pointercancel` before the finger reaches the dismiss threshold.
            div {
                class: "flex-1 min-h-0 flex flex-col select-none touch-none",
                onpointerdown: on_drag_start,

                // Art + title + podcast — shrink to fill the space above the
                // controls so nothing ever needs to scroll.
                div { class: "flex-1 min-h-0 flex flex-col items-center justify-center gap-3 px-6 pb-2 w-full",
                    // Art: the largest square that fits the remaining width AND
                    // height. `object-contain` + both `max-*` keep it square in any
                    // orientation and at any UI font scale (it just gets smaller).
                    div { class: "flex-1 min-h-0 w-full flex items-center justify-center",
                        Artwork {
                            src: art,
                            placeholder_src: art_small,
                            alt: "Episode artwork",
                            img_class: "max-h-full max-w-full object-contain rounded-2xl shadow-2xl",
                            placeholder_class: "text-6xl text-base-content/30",
                        }
                    }
                    h1 { class: "text-xl font-bold text-center line-clamp-2 shrink-0", "{title}" }
                    p { class: "text-base text-muted text-center line-clamp-1 shrink-0", "{podcast}" }
                }
            }

            // Controls — pinned at the bottom, centered + width-capped on desktop.
            div { class: "shrink-0 w-full max-w-xl mx-auto px-6 pb-4 space-y-3",
                // Current chapter — shown above the scrubber, updates as playback
                // crosses each marker. `Marquee` scrolls a long title in place.
                if let Some(idx) = active_chapter {
                    div { class: "text-xs text-muted",
                        Marquee { text: format!("{}. {}", idx + 1, chapters[idx].title) }
                    }
                }
                // Seek bar — chapter ticks overlaid on the native range input. The
                // ticks are decorative (`pointer-events-none`) so seeking is intact;
                // the thumb renders above them.
                div { class: "w-full",
                    div { class: "relative w-full",
                        input {
                            "aria-label": "Seek",
                            r#type: "range",
                            class: "w-full relative",
                            min: "0",
                            max: "{duration}",
                            step: "1",
                            value: "{shown_pos}",
                            disabled: np.duration_secs.is_none(),
                            oninput: move |e| {
                                if let Ok(secs) = e.value().parse::<f64>() {
                                    scrubbing.set(true);
                                    scrub_pos.set(secs);
                                }
                            },
                            onchange: move |e| {
                                let was_scrubbing = *scrubbing.peek();
                                scrubbing.set(false);
                                if was_scrubbing {
                                    // Seek from the tracked scrub position, not `e.value()` —
                                    // the input's value may have been programmatically
                                    // overwritten by a position tick just before release.
                                    player().seek_to(*scrub_pos.peek());
                                } else if let Ok(secs) = e.value().parse::<f64>() {
                                    player().seek_to(secs);
                                }
                            },
                            // The browser claimed the gesture — no `change` will fire;
                            // release the guard so ticks drive the thumb again.
                            onpointercancel: move |_| scrubbing.set(false),
                        }
                        if has_ticks {
                            for ch in chapters.iter() {
                                {
                                    let pct = (ch.starts_at_secs as f64 / duration * 100.0)
                                        .clamp(0.0, 100.0);
                                    rsx! {
                                        div {
                                            class: "absolute top-0 h-full w-px bg-base-content/40 pointer-events-none",
                                            style: "left: {pct}%",
                                        }
                                    }
                                }
                            }
                        }
                    }
                    div { class: "flex justify-between text-sm text-muted mt-1",
                        // Follows the finger while scrubbing, the live position otherwise.
                        span { "{format_time(shown_pos)}" }
                        if np.duration_secs.is_some() {
                            span { "{format_time(duration)}" }
                        } else {
                            span { "--:--" }
                        }
                    }
                }

                if np.state == PlaybackState::Preparing {
                    div { class: "flex items-center justify-center gap-2 text-sm text-muted",
                        span { class: "loading loading-spinner loading-sm" }
                        "Preparing download…"
                    }
                }
                if matches!(np.state, PlaybackState::Error(_)) {
                    p { class: "text-error text-sm text-center", "Playback error" }
                }

                // Transport controls. Track buttons drive queue/podcast-order
                // navigation directly — the Bluetooth next/prev override only
                // remaps hardware buttons, never these.
                div { class: "flex items-center justify-center gap-6",
                    button {
                        "aria-label": "Previous track",
                        class: "text-base-content/70 hover:text-base-content disabled:opacity-30 disabled:hover:text-base-content/70",
                        disabled: !has_prev(),
                        onclick: move |_| player().play_previous_episode(),
                        BackwardStep { class: "w-5 h-5" }
                    }
                    button {
                        class: "text-muted hover:text-base-content text-sm",
                        onclick: move |_| player().seek_relative(-(skip_backward as f64)),
                        "-{skip_backward}s"
                    }
                    button {
                        "aria-label": if np.state == PlaybackState::Playing { "Pause" } else { "Play" },
                        class: "w-16 h-16 bg-primary rounded-full flex items-center justify-center text-primary-content shadow-lg",
                        onclick: move |_| player().toggle(),
                        if np.state == PlaybackState::Playing {
                            Pause { class: "w-8 h-8" }
                        } else {
                            Play { class: "w-8 h-8" }
                        }
                    }
                    button {
                        class: "text-muted hover:text-base-content text-sm",
                        onclick: move |_| player().seek_relative(skip_forward as f64),
                        "+{skip_forward}s"
                    }
                    button {
                        "aria-label": "Next track",
                        class: "text-base-content/70 hover:text-base-content disabled:opacity-30 disabled:hover:text-base-content/70",
                        disabled: !has_next(),
                        onclick: move |_| player().play_next_episode(),
                        ForwardStep { class: "w-5 h-5" }
                    }
                }

                // Secondary controls. A 3-column `1fr auto 1fr` grid keeps the
                // auto-advance toggle dead-center on screen even though the side
                // controls have unequal widths; the side groups hug inward toward it.
                div {
                    class: "grid items-center gap-1",
                    style: "grid-template-columns: 1fr auto 1fr;",
                    // Left — chapters (when present) + playback speed, hugging center.
                    div { class: "flex items-center justify-end gap-1",
                        // Chapters — a drop-UP menu (this row sits at the bottom of
                        // the screen) listing the episode's markers; selecting one
                        // seeks there. Only shown when the episode has chapters.
                        if !chapters.is_empty() {
                            div { class: "dropdown dropdown-top",
                                div {
                                    tabindex: 0,
                                    role: "button",
                                    class: "btn btn-sm btn-ghost",
                                    "aria-label": "Chapters",
                                    ListUl { class: "w-4 h-4" }
                                }
                                ul {
                                    tabindex: 0,
                                    class: "dropdown-content menu bg-base-200 rounded-box p-1 shadow z-[70] mb-2 w-64 max-h-72 flex-nowrap overflow-y-auto",
                                    for (i, ch) in chapters.iter().enumerate() {
                                        li {
                                            button {
                                                class: if active_chapter == Some(i) { "active" } else { "" },
                                                onclick: {
                                                    let start = ch.starts_at_secs;
                                                    move |_| player().seek_to(start as f64)
                                                },
                                                span {
                                                    class: "text-xs text-muted tabular-nums mr-2",
                                                    "{format_time(ch.starts_at_secs as f64)}"
                                                }
                                                span { class: "truncate text-left", "{ch.title}" }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        // Speed — an icon trigger whose menu opens UPWARD (`dropdown-top`),
                        // since this control sits at the very bottom of the screen. The
                        // current rate rides next to the icon so it's visible at a glance.
                        div { class: "dropdown dropdown-top",
                            div {
                                tabindex: 0,
                                role: "button",
                                class: "btn btn-sm btn-ghost gap-1",
                                "aria-label": "Playback speed",
                                Bolt { class: "w-4 h-4" }
                                span { class: "text-xs", "{np.rate}x" }
                            }
                            ul {
                                tabindex: 0,
                                class: "dropdown-content menu bg-base-200 rounded-box p-1 shadow z-[70] mb-2 w-24",
                                for r in PLAYBACK_RATES {
                                    li {
                                        button {
                                            class: if np.rate == r { "active" } else { "" },
                                            onclick: move |_| {
                                                player().set_rate(r);
                                                // Persist as the new default so the choice sticks
                                                // across tracks (the controller applies
                                                // `playback_rate` on every load). Matches Settings.
                                                if config.peek().playback_prefs.playback_rate != r {
                                                    persist_config(config, |c| c.playback_prefs.playback_rate = r);
                                                }
                                            },
                                            "{r}x"
                                        }
                                    }
                                }
                            }
                        }
                    }
                    // Center — the auto-advance toggle ("A"), kept dead-center.
                    // Circled "A" when on, struck-through when off; mirrors the
                    // Settings checkbox and persists immediately.
                    button {
                        class: if auto_advance { "btn btn-sm btn-ghost" } else { "btn btn-sm btn-ghost text-base-content/40" },
                        "aria-label": "Auto-play next in queue",
                        "aria-pressed": "{auto_advance}",
                        onclick: move |_| {
                            persist_config(config, |c| c.playback_prefs.auto_advance = !auto_advance);
                        },
                        if auto_advance {
                            CircleA { class: "w-5 h-5" }
                        } else {
                            CircleAOff { class: "w-5 h-5" }
                        }
                    }
                    // Right — sleep timer, hugging center.
                    div { class: "flex items-center justify-start gap-1",
                        SleepTimerButton {}
                    }
                }
            }
        }
    }
}

/// Index of the chapter currently playing — the last marker whose start is at or
/// before `position_secs`. `None` before the first chapter's start (or when there
/// are no chapters). Assumes `chapters` is ordered by start (the server returns
/// them sorted by `starts_at_secs`).
fn active_chapter_index(chapters: &[EpisodeChapterData], position_secs: f64) -> Option<usize> {
    chapters
        .iter()
        .rposition(|c| c.starts_at_secs as f64 <= position_secs)
}

#[cfg(test)]
mod tests {
    use super::active_chapter_index;
    use chrono::Utc;
    use halogen_wire::EpisodeChapterData;

    fn chapter(starts_at_secs: i32) -> EpisodeChapterData {
        EpisodeChapterData {
            id: starts_at_secs,
            episode_id: 1,
            title: format!("Ch @{starts_at_secs}"),
            starts_at_secs,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn active_chapter_index_picks_last_started() {
        let chapters = [chapter(0), chapter(60), chapter(600)];
        // Before the first start (only possible if chapter 0 isn't at 0): none.
        assert_eq!(active_chapter_index(&[chapter(30)], 10.0), None);
        // Exactly on a start counts as that chapter.
        assert_eq!(active_chapter_index(&chapters, 0.0), Some(0));
        assert_eq!(active_chapter_index(&chapters, 60.0), Some(1));
        // Between starts → the earlier chapter.
        assert_eq!(active_chapter_index(&chapters, 59.9), Some(0));
        assert_eq!(active_chapter_index(&chapters, 300.0), Some(1));
        // Past the last start → the last chapter.
        assert_eq!(active_chapter_index(&chapters, 9_999.0), Some(2));
        // No chapters → none.
        assert_eq!(active_chapter_index(&[], 42.0), None);
    }
}
