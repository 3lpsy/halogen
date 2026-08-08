//! Small self-contained pieces of an episode row, shared by the list item and
//! the episode detail page so the same markup (classes / aria / glyph logic)
//! lives in one place. Each takes minimal props derived from
//! [`EpisodeRowState`](super::EpisodeRowState) plus the per-scope handles
//! (`player` / `dispatch`) the buttons fire through.

use dioxus::prelude::*;

use super::item::{CloudProgressOrSpinner, DownloadBadge, ProgressOrSpinner, download_badge};
use halogen_ui_commands::Command;
use halogen_ui_icons::{
    CircleCheck, CircleHalf, CloudArrowDown, Download, ForwardStep, Pause, Play, Trash,
};
use halogen_ui_state::commands;
use halogen_ui_svc_player::PlayerController;

/// The playback status glyph cluster shown to the right of the description: in
/// the queue the "up next" arrow takes priority (it's what plays when the
/// current episode ends) over the finished check / in-progress half-circle.
/// `role=img` + aria-label so each is announced (the icon is the only content).
#[component]
pub fn PlaybackMarker(is_next_up: bool, finished: bool, in_progress: bool) -> Element {
    rsx! {
        if is_next_up {
            span {
                class: "flex items-center justify-center px-2 text-primary",
                role: "img",
                "aria-label": "Up next",
                title: "Up next in queue",
                ForwardStep { class: "w-4 h-4" }
            }
        } else if finished {
            span {
                class: "flex items-center justify-center px-2 text-success",
                role: "img",
                "aria-label": "Finished",
                title: "Finished",
                CircleCheck { class: "w-4 h-4" }
            }
        } else if in_progress {
            span {
                class: "flex items-center justify-center px-2 text-success",
                role: "img",
                "aria-label": "Started",
                title: "Started — not finished",
                CircleHalf { class: "w-4 h-4" }
            }
        }
    }
}

/// The play/pause + length badge button. Disabled (greyed) when play is gated
/// off (`play_disabled`); shows the spinner while the player is preparing this
/// episode, a pause glyph while playing it, else the play glyph. Toggling
/// pauses/resumes the current episode or requests a fresh play — with
/// `playlist_id` (the list the row sits in, when it's a playlist/queue) as the
/// play context, so playback continues through that list.
#[component]
pub fn PlayBadge(
    episode_id: i32,
    is_current: bool,
    is_playing: bool,
    is_preparing: bool,
    play_disabled: bool,
    #[props(default)] playlist_id: Option<i32>,
    player: Signal<PlayerController>,
) -> Element {
    let play_badge_class = if play_disabled {
        "badge badge-outline gap-1 opacity-40"
    } else {
        "badge badge-outline gap-1 cursor-pointer"
    };
    rsx! {
        button {
            "aria-label": if is_playing { "Pause" } else { "Play" },
            class: "{play_badge_class}",
            disabled: play_disabled,
            onpointerdown: move |e: PointerEvent| e.stop_propagation(),
            onpointerup: move |e: PointerEvent| e.stop_propagation(),
            onclick: move |_| {
                if is_current { player().toggle(); } else { player().request_play_in(episode_id, playlist_id); }
            },
            if is_preparing {
                // Sized to the Play/Pause glyphs so the badge doesn't grow mid-swap.
                span { class: "loading loading-spinner w-3 h-3" }
            } else if is_playing {
                Pause { class: "w-3 h-3" }
            } else {
                Play { class: "w-3 h-3" }
            }
        }
    }
}

/// The device-download toggle badge. Honest states: spinner while bytes are
/// ACTUALLY being fetched to this device (also covers waiting for the server's
/// copy), trash once stored. `Failed` renders as the not-downloaded icon —
/// clicking retries. Disabled while in flight. The glyph is chosen by the pure
/// [`download_badge`] helper so the phase transition stays unit-testable.
#[component]
pub fn DeviceDownloadBadge(
    episode_id: i32,
    downloaded_on_device: bool,
    device_downloading: bool,
    device_download_progress: Option<u8>,
    server_downloading: bool,
    server_download_progress: Option<u8>,
    server_downloaded: bool,
    /// Embedded-server mode: the badge toggles the SERVER download (the only
    /// download concept left) instead of the device copy.
    embedded: bool,
    dispatch: Coroutine<Command>,
) -> Element {
    rsx! {
        button {
            class: "badge badge-ghost cursor-pointer",
            disabled: if embedded { server_downloading } else { device_downloading },
            title: if embedded {
                if server_downloaded {
                    "Downloaded — remove"
                } else if server_downloading {
                    "Downloading…"
                } else {
                    "Download"
                }
            } else if downloaded_on_device {
                "Downloaded on device — remove"
            } else if device_downloading {
                "Downloading to device…"
            } else if server_downloading {
                "Downloading on server…"
            } else if server_downloaded {
                "On server — download to device"
            } else {
                "Download to device"
            },
            onpointerdown: move |e: PointerEvent| e.stop_propagation(),
            onpointerup: move |e: PointerEvent| e.stop_propagation(),
            onclick: move |_| {
                if embedded {
                    if server_downloaded {
                        commands::remove_server_download(&dispatch, vec![episode_id]);
                    } else if !server_downloading {
                        commands::download_on_server(&dispatch, vec![episode_id]);
                    }
                } else if downloaded_on_device {
                    commands::remove_download(&dispatch, vec![episode_id]);
                } else if !device_downloading {
                    commands::download_to_device(&dispatch, vec![episode_id]);
                }
            },
            {
                match download_badge(
                    embedded,
                    downloaded_on_device,
                    device_downloading,
                    device_download_progress,
                    server_downloading,
                    server_download_progress,
                    server_downloaded,
                ) {
                    DownloadBadge::Remove => rsx! { Trash { class: "w-3 h-3" } },
                    // Server fetching its copy first: a cloud ring
                    // (spinner+cloud until the percent is known).
                    DownloadBadge::ServerFetching(p) => rsx! { CloudProgressOrSpinner { percent: p } },
                    // Bytes landing on THIS device: the plain ring
                    // (spinner until the first byte).
                    DownloadBadge::DeviceFetching(p) => rsx! { ProgressOrSpinner { percent: p } },
                    DownloadBadge::OnServer => rsx! { CloudArrowDown { class: "w-3 h-3" } },
                    DownloadBadge::Idle => rsx! { Download { class: "w-3 h-3" } },
                }
            }
        }
    }
}
