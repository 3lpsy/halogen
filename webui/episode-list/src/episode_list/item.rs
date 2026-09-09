use dioxus::prelude::*;
use halogen_wire::PlaybackStatus;

use super::action::{EpisodeActionCtx, perform_episode_action};
use super::row_parts::{DeviceDownloadBadge, PlayBadge, PlaybackMarker};
use super::row_state::{EpisodeRowState, MenuListContext, resolve_episode_row_state};
use crate::episode_menu::{EpisodeMenuArgs, episode_menu_sections};
use halogen_webui_app_state::media_url;
use halogen_webui_commands::actions as commands;
use halogen_webui_component_icons::{ChevronRight, CloudArrowDown, GripVertical};
use halogen_webui_component_widgets::{Artwork, start_drag_reorder, use_confirm, use_quick_menu};
use halogen_webui_hooks::{
    ToastLevel, use_config, use_connection, use_dispatch, use_downloads, use_episodes,
    use_now_playing_identity, use_play_context, use_playbacks, use_player_controller,
    use_playlists, use_podcasts, use_toast,
};
use halogen_webui_listview::{EpisodeData, SwipeAction, SwipeConfig};
use halogen_webui_logging::debug;

/// Pixels of horizontal drag past which a swipe action fires.
const SWIPE_THRESHOLD: f64 = 80.0;

/// Pixels of movement before the gesture commits to an axis (swipe vs. scroll).
/// Below this the gesture is undecided and the card doesn't move, so a vertical
/// scroll never nudges the row sideways.
const AXIS_THRESHOLD: f64 = 8.0;

/// How long the swipe-action confirmation toast lingers (ms). Short on purpose —
/// it's a "that happened" cue, not a message to read.
const SWIPE_TOAST_MS: u32 = 2_500;

/// The committed direction of an in-progress row gesture. `Undecided` until
/// movement passes [`AXIS_THRESHOLD`]; then either a horizontal swipe (the card
/// tracks the finger) or `Scroll` (vertical-dominant — the swipe is dropped so
/// the browser scrolls cleanly).
#[derive(Clone, Copy, PartialEq, Eq)]
enum SwipeAxis {
    Undecided,
    Horizontal,
    Scroll,
}

/// Decide the gesture axis from its displacement, or `None` while still under the
/// commit threshold. `Some(true)` = horizontal-dominant (a swipe); `Some(false)`
/// = vertical-dominant (a scroll). Pulled out as a free function so the
/// swipe-vs-scroll rule is unit-testable without a renderer.
fn decide_axis(dx: f64, dy: f64, threshold: f64) -> Option<bool> {
    if dx.abs().max(dy.abs()) < threshold {
        return None;
    }
    Some(dx.abs() > dy.abs())
}

/// PartialEq projection of a row's live playback state. Gating the row's
/// re-render on this (rather than reading `playbacks`/`EpisodeState` directly in
/// the render body) keeps a bursty worker publish from reflowing every visible
/// row — only rows whose finished/in-progress/progress actually changed re-render.
#[derive(Clone, Copy, PartialEq)]
struct PlaybackProj {
    finished: bool,
    in_progress: bool,
    progress_pct: Option<i32>,
}

/// A rich episode card. Left: optional reorder grip (Custom order only) + art. Right: title (2-line clamp), description
/// (1-line clamp), then a controls row (play/pause + length, device-download toggle, release date, kebab menu) and a
/// far-right chevron. Navigation to the episode page is via the title or the chevron only, tapping the card body does
/// NOT navigate or play. Swipe gestures remain; when `reorder_enabled` the left grip drags to reorder.
#[component]
pub fn EpisodeListItem(
    episode: EpisodeData,
    progress: Option<f32>,
    swipe: SwipeConfig,
    /// Manual (Custom) order is active for this list → show the drag grip and
    /// enable the Move actions. False greys the Move actions and hides the grip.
    #[props(default)]
    reorder_enabled: bool,
    /// This row's index within the rendered (Custom-ordered) list.
    #[props(default)]
    position: usize,
    /// Total rendered rows — boundary for Move Down / Move Last.
    #[props(default)]
    list_len: usize,
    /// The displayed list is in descending Custom order (down arrow), so reorder
    /// targets are mirrored back to stored ascending positions before dispatch.
    #[props(default)]
    reversed: bool,
    /// Full (un-windowed) pool length — the axis used to mirror reorder targets
    /// when `reversed`.
    #[props(default)]
    full_len: usize,
    /// The playlist this row belongs to, when the list is a playlist/queue.
    #[props(default)]
    playlist_id: Option<i32>,
    /// Multiselect mode is on for this list → show the selection checkbox (in place
    /// of the reorder grip) and disable swipe gestures.
    #[props(default)]
    multiselect_active: bool,
    /// Whether this row is currently ticked.
    #[props(default)]
    selected: bool,
    /// Toggle this row's selection.
    on_toggle_select: Callback<i32>,
) -> Element {
    // Read worker signals through PartialEq memos so publications recompute cheap projections without rerendering
    // unchanged rows. New raw reads in the body would restore render storms; direct config reads are safe because
    // settings changes are infrequent.
    let app_state = use_episodes();
    let podcasts = use_podcasts();
    let playlists = use_playlists();
    let playbacks = use_playbacks();
    let downloads = use_downloads();
    let connection = use_connection();
    let player = use_player_controller();
    let playing = use_now_playing_identity();
    let dispatch = use_dispatch();
    let nav = use_navigator();
    let quick = use_quick_menu();
    let confirm = use_confirm();
    let config = use_config();
    let toast = use_toast();

    let episode_id = episode.id;
    let podcast_id = episode.podcast_id;
    let title = episode.title.clone();
    // Owned copy for the context-menu header (kept separate from the borrowed
    // `{title}` interpolation in the card).
    let menu_title = title.clone();
    // Memoize the current podcast name from its pool, falling back to embedded episode data. Missing names trigger lazy
    // fetch and show a placeholder; unrelated podcast publications do not rerender the row.
    let prop_podcast_name = episode.podcast.as_ref().map(|p| p.title.clone());
    let podcast_name = use_memo(move || {
        podcasts
            .read()
            .podcast(podcast_id)
            .map(|p| p.title.clone())
            .or_else(|| prop_podcast_name.clone())
            .filter(|s| !s.is_empty())
    });
    // Lazily pull the parent podcast into the pool if its name isn't there yet
    // (the worker de-dups, so a list full of the same podcast fetches once).
    use_effect(move || {
        if podcasts.peek().podcast(podcast_id).is_none() {
            commands::ensure_podcast(&dispatch, podcast_id);
        }
    });
    // Feed descriptions are HTML; show a plain-text (tags stripped) preview.
    let description = episode
        .description
        .as_deref()
        .map(halogen_webui_component_widgets::html::to_plain)
        .filter(|s| !s.is_empty());
    // Artwork via the SERVER art cache — never the feed's origin URL (the
    // frontend CSP would block it anyway). The list renders ~70px tiles, so it
    // pulls the downscaled `/art/small` thumbnail, not the multi-MB original.
    let image_url =
        media_url::art_url_for_episode_small(config.read().server_url.as_deref(), &episode);
    let published = episode
        .published_at
        .map(|d| d.format("%b %d").to_string())
        .unwrap_or_else(|| "—".to_string());
    let minutes = episode.duration_secs.map(|s| (s / 60).max(1));

    // Use the shared row-state builder for list/detail consistency. Its memo reads equality-gated playing identity
    // rather than 250ms position updates, and its PartialEq result suppresses unchanged renders.
    let row_memo = use_memo(move || {
        resolve_episode_row_state(
            &app_state.read(),
            &playlists.read(),
            &downloads.read(),
            &connection.read(),
            &playing.read(),
            episode_id,
        )
    });
    let row = row_memo();
    let EpisodeRowState {
        downloaded_on_device,
        server_downloaded,
        server_downloading,
        server_download_progress,
        device_downloading,
        device_download_progress,
        queue_id,
        in_queue,
        is_current,
        is_playing,
        is_preparing,
        play_disabled,
        ..
    } = row;
    // Mark next-up only within the active continuation playlist, falling back to the queue when context membership is
    // unknown. Memoize the target so playback position ticks cannot rerender rows.
    let play_context = use_play_context();
    let is_next_up = use_memo(move || {
        let pl = playlists.read();
        let ctx = play_context.read().0;
        let effective = ctx
            .filter(|id| pl.episodes_by_playlist.contains_key(id))
            .or_else(|| pl.queue_id());
        if playlist_id.is_none() || playlist_id != effective {
            return false;
        }
        let current = playing.read().as_ref().map(|n| n.episode_id);
        pl.next_up_in(current, ctx) == Some(episode_id)
    });
    // Memoize finished/progress state from live playback_for, falling back to stale episode props only without an
    // overlay. Marker and bar share one projection so they agree; PartialEq suppresses unchanged renders during cursor
    // updates.
    let prop_status = episode.playback_status;
    let prop_duration = episode.duration_secs;
    let playback_proj = use_memo(move || {
        let live_pb = playbacks.read().playback_for(&app_state.read(), episode_id);
        let finished = live_pb.as_ref().is_some_and(|p| p.completed)
            || prop_status == PlaybackStatus::Finished;
        let started = live_pb.as_ref().is_some_and(|p| p.cursor > 0);
        let in_progress = !finished && (started || prop_status == PlaybackStatus::Played);
        let live_progress = live_pb.as_ref().and_then(|pb| {
            let duration = prop_duration.filter(|d| *d > 0)?;
            let frac = (pb.cursor as f64 / duration as f64).clamp(0.0, 1.0) as f32;
            (frac > 0.0 && frac < 1.0).then_some(frac)
        });
        let progress_pct = live_progress
            .or(progress)
            .map(|p| (p.clamp(0.0, 1.0) * 100.0) as i32);
        PlaybackProj {
            finished,
            in_progress,
            progress_pct,
        }
    });
    let PlaybackProj {
        finished,
        in_progress,
        progress_pct,
    } = playback_proj();
    // `completed` backs the `TogglePlayed` swipe — an alias of `finished`.
    let completed = finished;

    // The quick-context-menu sections are built AT OPEN TIME inside the kebab's onclick (below), not here in the render
    // body: the builder allocates ~7 sections of `Rc` closures that are only consumed if the kebab opens, and this row
    // re-renders on every relevant publish (~1 Hz during a download), building them per render was pure waste on the
    // hottest path.

    // Drag state for swipe. `drag_start` holds the pointer-down (x, y); `axis`
    // tracks the swipe-vs-scroll decision; `offset` is the live horizontal shift.
    let mut drag_start = use_signal(|| None::<(f64, f64)>);
    let mut axis = use_signal(|| SwipeAxis::Undecided);
    let mut offset = use_signal(|| 0.0_f64);

    let swipe_left = swipe.left;
    let swipe_right = swipe.right;

    // Resolve swipes through the same live-state dispatcher as menu actions. In local-only mode, badges, menus, and
    // swipes use the runtime's single download set.
    let embedded = config.read().server_kind.is_embedded();
    let action_ctx = EpisodeActionCtx {
        episode_id,
        queue_id,
        playlist_id,
        in_queue,
        downloaded_on_device,
        server_downloaded,
        completed,
        embedded,
    };
    let run_swipe = move |action: SwipeAction| {
        // Confirm the action took place — there's otherwise little feedback that a
        // swipe fired. Brief info toast; the queue auto-dismisses + dedups. No
        // toast when the dispatcher no-ops (e.g. a queue action before the queue
        // resolves) — confirming an action that never ran is worse than silence.
        if perform_episode_action(action, &action_ctx, dispatch, player, nav) {
            toast.show(ToastLevel::Info, action.label(), Some(SWIPE_TOAST_MS));
        }
    };

    let onpointerdown = move |e: PointerEvent| {
        // Multiselect: no swipe — the row's gesture is reserved for selection.
        if multiselect_active {
            return;
        }
        let c = e.client_coordinates();
        drag_start.set(Some((c.x, c.y)));
        axis.set(SwipeAxis::Undecided);
        offset.set(0.0);
    };
    let onpointermove = move |e: PointerEvent| {
        let Some((sx, sy)) = drag_start() else { return };
        // No button held during a tracked drag → the pointer was released OUTSIDE this row (a mouse-up off the element
        // never fires our `pointerup`), so the gesture is over. Without this the drag state latched: a later plain
        // hover kept matching `drag_start` and dragged the card around under an unpressed cursor. Reset and stop
        // tracking.
        if e.held_buttons().is_empty() {
            drag_start.set(None);
            axis.set(SwipeAxis::Undecided);
            offset.set(0.0);
            return;
        }
        let c = e.client_coordinates();
        let dx = c.x - sx;
        match axis() {
            // Locked to a horizontal swipe: the card tracks the finger.
            SwipeAxis::Horizontal => offset.set(dx),
            // Committed to a scroll: leave the card put, let the browser scroll.
            SwipeAxis::Scroll => {}
            // Undecided — commit to an axis once past the threshold. Vertical
            // wins → drop the swipe so a scroll never drags the row sideways.
            SwipeAxis::Undecided => match decide_axis(dx, c.y - sy, AXIS_THRESHOLD) {
                Some(true) => {
                    axis.set(SwipeAxis::Horizontal);
                    offset.set(dx);
                }
                Some(false) => axis.set(SwipeAxis::Scroll),
                None => {}
            },
        }
    };
    let onpointerup = move |e: PointerEvent| {
        let Some((sx, _)) = drag_start() else { return };
        let dx = e.client_coordinates().x - sx;
        let was_swipe = axis() == SwipeAxis::Horizontal;
        drag_start.set(None);
        axis.set(SwipeAxis::Undecided);
        offset.set(0.0);
        // Only a committed horizontal swipe past the threshold fires an action — a
        // plain tap or a scroll does nothing (title/chevron own navigation).
        if !was_swipe {
            return;
        }
        if dx > SWIPE_THRESHOLD {
            if let Some(a) = swipe_left {
                run_swipe(a);
            }
        } else if dx < -SWIPE_THRESHOLD
            && let Some(a) = swipe_right
        {
            run_swipe(a);
        }
    };
    // The browser claimed the gesture (scroll/route change) → it fires
    // `pointercancel`, not `pointerup`. Reset so the card never sticks half-swiped.
    let onpointercancel = move |_: PointerEvent| {
        drag_start.set(None);
        axis.set(SwipeAxis::Undecided);
        offset.set(0.0);
    };

    // Grip drag-to-reorder (Custom order only) — see `start_drag_reorder`. The
    // reported target is the original `data-ep-index` of the row under the pointer
    // (a *visual* index); `move_in_playlist_visual` mirrors it back to a stored
    // ascending position when the list is shown descending.
    let on_grip_down = move |e: PointerEvent| {
        e.stop_propagation();
        if !reorder_enabled {
            return;
        }
        let Some(pid) = playlist_id else {
            return;
        };
        start_drag_reorder(
            "episode-scroll",
            "data-ep-index",
            &format!("ep-row-{episode_id}"),
            position,
            e.client_coordinates().y,
            move |to| {
                commands::move_in_playlist_visual(
                    &dispatch, pid, episode_id, to as i32, reversed, full_len,
                )
            },
        );
    };

    rsx!(
        div {
            class: "relative overflow-hidden",
            id: "ep-row-{episode_id}",
            "data-ep-index": "{position}",
            // Skip offscreen row layout while retaining DOM nodes. Match contain-intrinsic-size to the measured 8.9rem
            // row height so user font scaling preserves its estimate; this optimizes painting rather than fixing
            // whole-list cold-load reflows.
            style: "content-visibility: auto; contain-intrinsic-size: 8.9rem;",
            // Left-reveal action (triggered by swiping right). Each word stacks on
            // its own line and hugs the left edge, so a multi-word label (e.g.
            // "Remove from playlist") stays readable in a thin reveal instead of
            // disappearing behind the sliding card.
            if let Some(a) = swipe_left {
                div {
                    class: "absolute inset-y-0 left-0 flex flex-col justify-center items-start px-2 bg-base-300 text-base-content text-sm font-medium leading-tight",
                    for word in a.label().split_whitespace() {
                        div { key: "{word}", "{word}" }
                    }
                }
            }
            // Right-reveal action (triggered by swiping left). Mirror of the left
            // reveal: words stack and hug the right edge.
            if let Some(a) = swipe_right {
                div {
                    class: "absolute inset-y-0 right-0 flex flex-col justify-center items-end px-2 bg-base-300 text-base-content text-sm font-medium leading-tight text-right",
                    for word in a.label().split_whitespace() {
                        div { key: "{word}", "{word}" }
                    }
                }
            }
            // The draggable card. The snap-back transition is applied only when
            // NOT actively swiping, so a release glides back to 0 while an
            // in-progress swipe still tracks the finger 1:1.
            div {
                class: if axis() == SwipeAxis::Horizontal {
                    "card bg-base-100 shadow-sm border border-base-300 relative z-10 select-none"
                } else {
                    "card bg-base-100 shadow-sm border border-base-300 relative z-10 select-none transition-transform duration-200"
                },
                style: "transform: translateX({offset()}px); touch-action: pan-y;",
                onpointerdown,
                onpointermove,
                onpointerup,
                onpointercancel,
                div { class: "card-body p-3",
                    div { class: "flex items-stretch gap-3",
                        // Multiselect checkbox — sits where the grip would (the two
                        // are mutually exclusive: reorder is off in multiselect). Matches
                        // the grip's column width exactly (`-ml-1 pr-1` + a `w-4`-sized
                        // `checkbox-xs`) so swapping grip↔checkbox doesn't shift the row.
                        if multiselect_active {
                            label {
                                class: "flex items-center -ml-1 pr-1 cursor-pointer",
                                onpointerdown: move |e: PointerEvent| e.stop_propagation(),
                                onpointerup: move |e: PointerEvent| e.stop_propagation(),
                                input {
                                    r#type: "checkbox",
                                    class: "checkbox checkbox-primary checkbox-xs",
                                    checked: selected,
                                    onchange: move |_| on_toggle_select.call(episode_id),
                                }
                            }
                        }
                        // Reorder grip (Custom order only), press-and-hold, then drag to move the row. `touch-action:
                        // pan-y` lets a quick flick on the grip scroll the list normally; the reorder only engages
                        // after a short hold (see `start_drag_reorder`). `stop_propagation` keeps the card's swipe from
                        // firing so only the grip starts a vertical drag.
                        if reorder_enabled {
                            div {
                                id: "ep-grip-{episode_id}",
                                // Tagged so the pull-to-refresh shim
                                // (use_pull_to_refresh) ignores a gesture that
                                // begins on the grip — otherwise dragging a row
                                // down from the top arms a refresh/poll mid-reorder.
                                "data-reorder-grip": "1",
                                class: "flex items-center cursor-grab text-base-content/30 hover:text-muted -ml-1 pr-1",
                                style: "touch-action: pan-y;",
                                "aria-label": "Drag to reorder",
                                onpointerdown: on_grip_down,
                                GripVertical { class: "w-4 h-4" }
                            }
                        }
                        // Content column — art + title/podcast on the top row; the
                        // description, controls, progress, and kebab all span the FULL
                        // width beneath it (including under the art).
                        div { class: "flex-1 min-w-0",
                            // Top row: art (left) + title/podcast + the episode chevron.
                            div { class: "flex items-start gap-3",
                                // Art — server art cache (optimistic; placeholder on miss).
                                div { class: "flex-shrink-0 w-14 h-14 rounded bg-base-200 flex items-center justify-center overflow-hidden",
                                    Artwork {
                                        src: image_url,
                                        alt: "Episode art",
                                        img_class: "w-full h-full object-cover",
                                        placeholder_class: "text-xl",
                                    }
                                }
                                // Title + podcast name (to the right of the art).
                                div { class: "flex-1 min-w-0",
                                    // Disable title navigation during multiselect. Keep h2 for heading order and e2e
                                    // selectors; reserve both clamped title lines so row height matches virtualization
                                    // estimates without layout shifts.
                                    h2 {
                                        class: if multiselect_active { "text-sm font-semibold leading-snug line-clamp-2 min-h-[2.4rem]" } else { "text-sm font-semibold leading-snug line-clamp-2 min-h-[2.4rem] cursor-pointer" },
                                        onpointerdown: move |e: PointerEvent| e.stop_propagation(),
                                        onpointerup: move |e: PointerEvent| e.stop_propagation(),
                                        onclick: move |_| {
                                            if !multiselect_active {
                                                nav.push(format!("/episodes/{episode_id}"));
                                            }
                                        },
                                        "{title}"
                                    }
                                    if let Some(podcast) = podcast_name() {
                                        // Podcast name links to its page — inert in
                                        // multiselect (mirrors the title above).
                                        p {
                                            class: if multiselect_active { "text-xs font-medium text-primary line-clamp-1 mt-0.5 w-fit" } else { "text-xs font-medium text-primary line-clamp-1 mt-0.5 cursor-pointer w-fit" },
                                            onpointerdown: move |e: PointerEvent| e.stop_propagation(),
                                            onpointerup: move |e: PointerEvent| e.stop_propagation(),
                                            onclick: move |_| {
                                                if !multiselect_active {
                                                    nav.push(format!("/podcasts/{podcast_id}"));
                                                }
                                            },
                                            "{podcast}"
                                        }
                                    } else {
                                        p { class: "text-xs font-medium text-muted line-clamp-1 mt-0.5", "…" }
                                    }
                                }
                                // Chevron → view episode: top-right, centered against the
                                // art, with the kebab landing directly beneath it. Hidden
                                // in multiselect (nav is inert there).
                                if !multiselect_active {
                                    button {
                                        "aria-label": "View episode",
                                        class: "self-stretch flex items-center justify-center px-2 text-base-content/30 hover:text-muted",
                                        onpointerdown: move |e: PointerEvent| e.stop_propagation(),
                                        onpointerup: move |e: PointerEvent| e.stop_propagation(),
                                        onclick: move |_| { nav.push(format!("/episodes/{episode_id}")); },
                                        ChevronRight { class: "w-5 h-5" }
                                    }
                                }
                            }
                            // Always reserve the description/finished-marker line so late playback state cannot grow
                            // the row. Align the marker with the chevron and kebab in the right-hand column.
                            div { class: "flex items-center gap-2 mt-1",
                                p { class: "flex-1 min-w-0 text-xs text-muted line-clamp-1 min-h-[1rem]",
                                    if let Some(desc) = description {
                                        "{desc}"
                                    }
                                }
                                // Playback marker. In the queue, the "up next" arrow takes priority, it's what plays
                                // when the current episode ends, over the finished check / in-progress half-circle. The
                                // thin progress bar below still shows position when in progress.
                                PlaybackMarker {
                                    is_next_up: is_next_up(),
                                    finished,
                                    in_progress,
                                }
                            }
                            // Controls + a thin in-progress bar beneath them, in a
                            // single row so the kebab can span both lines (a bigger
                            // tap target).
                            div { class: "flex items-stretch gap-2 mt-1.5",
                              div { class: "flex-1 min-w-0",
                                div { class: "flex items-center gap-2",
                                // Play/pause + length badge.
                                PlayBadge {
                                    episode_id,
                                    is_current,
                                    is_playing,
                                    is_preparing,
                                    play_disabled,
                                    playlist_id,
                                    player,
                                }
                                // Device-download toggle badge.
                                DeviceDownloadBadge {
                                    episode_id,
                                    downloaded_on_device,
                                    device_downloading,
                                    device_download_progress,
                                    server_downloading,
                                    server_download_progress,
                                    server_downloaded,
                                    embedded,
                                    dispatch,
                                }
                                // Duration, rounded to minutes — between the
                                // download/trash icon and the release date.
                                if let Some(m) = minutes {
                                    span { class: "text-xs text-muted", "{m} min" }
                                }
                                // Release date.
                                span { class: "text-xs text-muted", "{published}" }
                                }
                                // Reserve progress-track height even without a percentage so late playback data cannot
                                // shift rows. Show the line only for unfinished progress; it is an indicator, not a
                                // seek control.
                                div {
                                    class: if progress_pct.is_some() {
                                        "h-1 mt-1.5 bg-base-300 rounded-full overflow-hidden"
                                    } else {
                                        "h-1 mt-1.5 rounded-full overflow-hidden"
                                    },
                                    if let Some(pct) = progress_pct {
                                        div { class: "h-full bg-primary rounded-full", style: "width: {pct}%" }
                                    }
                                }
                              }
                              // Kebab → shared right slide-in menu (escapes the swipe
                              // card's transform/overflow). Spans both lines vertically
                              // for a larger tap target. Hidden in multiselect — the bulk
                              // menu (sub-bar pencil) replaces the per-row actions there.
                              if !multiselect_active {
                                  button {
                                      class: "self-stretch flex items-center justify-center px-2 text-lg leading-none rounded text-muted hover:text-base-content hover:bg-base-200",
                                      "aria-label": "Episode actions",
                                      onpointerdown: move |e: PointerEvent| e.stop_propagation(),
                                      onpointerup: move |e: PointerEvent| e.stop_propagation(),
                                      onclick: move |_| {
                                          debug!(episode_id, "Opened episode actions menu");
                                          // Built at open time from the live row state —
                                          // see the comment where `menu_title` is defined.
                                          let sections = episode_menu_sections(
                                              EpisodeMenuArgs::from_row_state(
                                                  &row_memo.read(),
                                                  podcast_id,
                                                  config.read().playback_prefs.playback_preference,
                                                  embedded,
                                                  MenuListContext {
                                                      playlist_id,
                                                      reorder_enabled,
                                                      position,
                                                      list_len,
                                                      reversed,
                                                      full_len,
                                                  },
                                                  true,
                                              ),
                                              player,
                                              dispatch,
                                              nav,
                                              confirm.purge_episode_callback(episode_id),
                                          );
                                          quick.open(menu_title.clone(), sections);
                                      },
                                      "⋯"
                                  }
                              }
                            }
                        }
                    }
                }
            }
        }
    )
}

/// The in-flight download glyph: a filling [`DownloadProgressRing`] once a percent
/// is known, else the indeterminate xs spinner. Shared by the device and server
/// download branches (and the detail page) — the only difference between them is
/// which percent signal feeds it, so the spinner-vs-ring choice lives here once.
#[component]
pub fn ProgressOrSpinner(percent: Option<u8>) -> Element {
    rsx! {
        if let Some(pct) = percent {
            DownloadProgressRing { percent: pct }
        } else {
            span { class: "loading loading-spinner loading-xs" }
        }
    }
}

/// The download badge's glyph state. Lives behind [`download_badge`] so the
/// cloud→device phase transition is testable without a renderer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DownloadBadge {
    /// On this device — tap removes it.
    Remove,
    /// The SERVER is fetching its copy (cloud ring); percent if the tracker knows it.
    ServerFetching(Option<u8>),
    /// Bytes are landing on THIS device (plain ring); percent once the first lands.
    DeviceFetching(Option<u8>),
    /// On the server but not this device — tap pulls it down.
    OnServer,
    /// Nothing yet — tap downloads.
    Idle,
}

/// Show the cloud phase while a requested device download awaits server bytes. Switch to the device ring once its
/// progress exists or the server already has the copy; retain a separate server-only download state.
pub(crate) fn download_badge(
    embedded: bool,
    downloaded_on_device: bool,
    device_downloading: bool,
    download_progress: Option<u8>,
    server_downloading: bool,
    server_download_progress: Option<u8>,
    server_downloaded: bool,
) -> DownloadBadge {
    // Embedded server: the server's copy IS the local copy — the badge tracks
    // only the server states, with the PLAIN ring (nothing is remote, so no
    // cloud phase), and "downloaded on server" reads as done (Remove).
    if embedded {
        return if server_downloaded {
            DownloadBadge::Remove
        } else if server_downloading {
            DownloadBadge::DeviceFetching(server_download_progress)
        } else {
            DownloadBadge::Idle
        };
    }
    if downloaded_on_device {
        DownloadBadge::Remove
    } else if device_downloading {
        if download_progress.is_none() && server_downloading {
            DownloadBadge::ServerFetching(server_download_progress)
        } else {
            DownloadBadge::DeviceFetching(download_progress)
        }
    } else if server_downloading {
        DownloadBadge::ServerFetching(server_download_progress)
    } else if server_downloaded {
        DownloadBadge::OnServer
    } else {
        DownloadBadge::Idle
    }
}

/// Server-fetch variant of [`ProgressOrSpinner`]: the same filling ring (or spinner
/// while the length is unknown) with a small cloud centered inside, marking this as
/// the SERVER copy being fetched — distinct from the bare device ring shown once the
/// byte-pull to this device begins.
#[component]
pub fn CloudProgressOrSpinner(percent: Option<u8>) -> Element {
    rsx! {
        span { class: "relative inline-flex items-center justify-center w-3 h-3",
            ProgressOrSpinner { percent }
            span { class: "absolute inset-0 flex items-center justify-center",
                CloudArrowDown { class: "w-1.5 h-1.5" }
            }
        }
    }
}

#[component]
fn DownloadProgressRing(percent: u8) -> Element {
    // r = 9 in a 24×24 box. Shrinking the dash offset from the full circumference
    // toward 0 reveals the arc; rotate -90° so it starts at 12 o'clock.
    const CIRCUMFERENCE: f32 = 2.0 * std::f32::consts::PI * 9.0;
    let offset = CIRCUMFERENCE * (1.0 - (percent.min(100) as f32) / 100.0);
    rsx! {
        svg {
            class: "w-3 h-3",
            view_box: "0 0 24 24",
            fill: "none",
            "aria-hidden": "true",
            circle {
                cx: "12",
                cy: "12",
                r: "9",
                stroke: "currentColor",
                stroke_width: "3",
                opacity: "0.25",
            }
            circle {
                cx: "12",
                cy: "12",
                r: "9",
                stroke: "currentColor",
                stroke_width: "3",
                stroke_linecap: "round",
                stroke_dasharray: "{CIRCUMFERENCE}",
                stroke_dashoffset: "{offset}",
                transform: "rotate(-90 12 12)",
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{AXIS_THRESHOLD, DownloadBadge, decide_axis, download_badge};

    /// Below the commit threshold the gesture stays undecided — the card must not
    /// move on a tiny jitter.
    #[test]
    fn axis_undecided_under_threshold() {
        assert_eq!(decide_axis(3.0, 2.0, AXIS_THRESHOLD), None);
        assert_eq!(decide_axis(-7.9, 0.0, AXIS_THRESHOLD), None);
    }

    /// Horizontal-dominant movement past the threshold commits to a swipe.
    #[test]
    fn axis_horizontal_dominant_is_swipe() {
        assert_eq!(decide_axis(20.0, 5.0, AXIS_THRESHOLD), Some(true));
        assert_eq!(decide_axis(-15.0, 10.0, AXIS_THRESHOLD), Some(true));
    }

    /// Vertical-dominant movement past the threshold commits to a scroll (swipe
    /// dropped) — the row never slides sideways while scrolling.
    #[test]
    fn axis_vertical_dominant_is_scroll() {
        assert_eq!(decide_axis(5.0, 20.0, AXIS_THRESHOLD), Some(false));
        assert_eq!(decide_axis(0.0, -12.0, AXIS_THRESHOLD), Some(false));
    }

    /// Tapping "download to device" sets `device_downloading` immediately. Until the
    /// device byte-pull starts (`download_pct() == None`) and while the server is
    /// still fetching its copy, the badge must show the SERVER (cloud) phase — not
    /// the bare device spinner that used to mask it.
    #[test]
    fn to_device_shows_server_phase_before_device_bytes() {
        // Just tapped: server fetch underway, no percent yet → cloud spinner.
        assert_eq!(
            download_badge(false, false, true, None, true, None, false),
            DownloadBadge::ServerFetching(None),
        );
        // Server reports progress → cloud ring carries the server percent.
        assert_eq!(
            download_badge(false, false, true, None, true, Some(40), false),
            DownloadBadge::ServerFetching(Some(40)),
        );
    }

    /// Once device bytes land, the badge flips to the plain device ring carrying the
    /// device percent — even if a late server-progress value lingers.
    #[test]
    fn device_bytes_take_over_the_ring() {
        assert_eq!(
            download_badge(false, false, true, Some(10), false, None, false),
            DownloadBadge::DeviceFetching(Some(10)),
        );
        assert_eq!(
            download_badge(false, false, true, Some(10), true, Some(99), false),
            DownloadBadge::DeviceFetching(Some(10)),
        );
    }

    /// When the server already holds the file, a to-device tap skips the cloud phase
    /// entirely: no server fetch, so it's the plain device ring/spinner from the off.
    #[test]
    fn server_already_has_it_skips_cloud_phase() {
        assert_eq!(
            download_badge(false, false, true, None, false, None, true),
            DownloadBadge::DeviceFetching(None),
        );
    }

    /// A server-ONLY download (no device pull queued) still shows the cloud ring.
    #[test]
    fn server_only_download_shows_cloud() {
        assert_eq!(
            download_badge(false, false, false, None, true, Some(55), false),
            DownloadBadge::ServerFetching(Some(55)),
        );
    }

    /// The static states are unchanged.
    #[test]
    fn static_states() {
        assert_eq!(
            download_badge(false, true, false, None, false, None, false),
            DownloadBadge::Remove,
        );
        assert_eq!(
            download_badge(false, false, false, None, false, None, true),
            DownloadBadge::OnServer,
        );
        assert_eq!(
            download_badge(false, false, false, None, false, None, false),
            DownloadBadge::Idle,
        );
    }

    /// Embedded mode: the badge tracks only the server states — downloaded reads
    /// as done (Remove, never the pull-down OnServer), an in-flight fetch shows
    /// the PLAIN ring with the server percent (no cloud phase — nothing is
    /// remote), and stale device flags from a previous remote life are ignored.
    #[test]
    fn embedded_badge_tracks_server_states_only() {
        assert_eq!(
            download_badge(true, false, false, None, false, None, true),
            DownloadBadge::Remove,
        );
        assert_eq!(
            download_badge(true, false, false, None, true, Some(55), false),
            DownloadBadge::DeviceFetching(Some(55)),
        );
        assert_eq!(
            download_badge(true, false, false, None, false, None, false),
            DownloadBadge::Idle,
        );
        // Stale device-download state must not leak into the embedded badge.
        assert_eq!(
            download_badge(true, true, true, Some(10), false, None, false),
            DownloadBadge::Idle,
        );
    }
}
