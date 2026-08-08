//! Single episode-action dispatcher.
//!
//! A configured [`SwipeAction`](halogen_ui_listview::SwipeAction) and the per-row kebab
//! menu both ultimately fire the same `commands::*` / player / nav primitives.
//! This module routes the swipe gesture through one place so the action→command
//! mapping isn't copied a third time alongside the menu builders.
//!
//! The toggle-style swipe actions (`ToggleQueue`, `ToggleDownload`,
//! `ToggleServerDownload`, `TogglePlayed`) resolve against the row's live flags
//! to the same add/remove primitives the menu offers as discrete actions.

use dioxus::prelude::*;
use dioxus::router::Navigator;

use halogen_ui_commands::Command;
use halogen_ui_listview::SwipeAction;
use halogen_ui_state::commands;
use halogen_ui_svc_player::PlayerController;

/// The live per-row context every episode action needs: the episode and the
/// list/queue it sits in, plus the toggle-resolving flags (queue membership,
/// device/server download state, completed). Carried so `perform_episode_action`
/// is a pure mapping with no `EpisodeState` reads of its own.
#[derive(Clone, Copy, Debug)]
pub struct EpisodeActionCtx {
    pub episode_id: i32,
    /// The queue (default playlist) id, when one exists.
    pub queue_id: Option<i32>,
    /// The playlist this row belongs to (queue or viewed playlist), when any.
    pub playlist_id: Option<i32>,
    pub in_queue: bool,
    pub downloaded_on_device: bool,
    pub server_downloaded: bool,
    /// Listened to the end — backs `TogglePlayed`.
    pub completed: bool,
    /// Embedded-server mode: the device-download swipe actions remap to their
    /// server counterparts (stored swipe configs keep working; only the target
    /// changes — the server IS this device).
    pub embedded: bool,
}

/// Perform one episode action against the row's live context, firing the same
/// `commands::*` / player / nav primitives the kebab menu uses. The single copy
/// of the action→command mapping for the gesture path.
///
/// Returns whether anything was actually dispatched — the queue/list actions
/// no-op when their target (`queue_id` / `playlist_id`) hasn't resolved or
/// doesn't exist, and the caller must not confirm an action that never ran.
pub fn perform_episode_action(
    action: SwipeAction,
    ctx: &EpisodeActionCtx,
    dispatch: Coroutine<Command>,
    player: Signal<PlayerController>,
    nav: Navigator,
) -> bool {
    let id = ctx.episode_id;
    // Fully-qualified (no `use SwipeAction::*`): `Play`/`Download` would clash
    // with the icon imports at the call site's module.
    match action {
        SwipeAction::Play => player().request_play_in(id, ctx.playlist_id),
        SwipeAction::Stream => player().stream_episode_in(id, ctx.playlist_id),
        SwipeAction::TogglePlayed => commands::mark_played(&dispatch, id, !ctx.completed),
        SwipeAction::MarkPlayed => commands::mark_played(&dispatch, id, true),
        SwipeAction::MarkUnplayed => commands::mark_played(&dispatch, id, false),
        SwipeAction::ResetProgress => commands::set_cursor(&dispatch, id, 0),
        SwipeAction::AddToQueue => {
            let Some(q) = ctx.queue_id else { return false };
            commands::add_to_playlist(&dispatch, q, vec![id]);
        }
        SwipeAction::RemoveFromQueue => {
            let Some(q) = ctx.queue_id else { return false };
            commands::remove_from_playlist(&dispatch, q, vec![id]);
        }
        SwipeAction::ToggleQueue => {
            let Some(q) = ctx.queue_id else { return false };
            if ctx.in_queue {
                commands::remove_from_playlist(&dispatch, q, vec![id]);
            } else {
                commands::add_to_playlist(&dispatch, q, vec![id]);
            }
        }
        SwipeAction::RemoveFromList => {
            let Some(pid) = ctx.playlist_id else {
                return false;
            };
            commands::remove_from_playlist(&dispatch, pid, vec![id]);
        }
        SwipeAction::AddToPlaylist => {
            nav.push(format!("/episodes/{id}/playlists"));
        }
        // Embedded remap: the device actions target the server download (the
        // only download concept in that mode) — see `EpisodeActionCtx::embedded`.
        SwipeAction::DownloadToDevice if ctx.embedded => {
            commands::download_on_server(&dispatch, vec![id])
        }
        SwipeAction::RedownloadDevice if ctx.embedded => {
            commands::redownload_on_server(&dispatch, vec![id])
        }
        SwipeAction::RemoveDownload if ctx.embedded => {
            commands::remove_server_download(&dispatch, vec![id])
        }
        SwipeAction::ToggleDownload if ctx.embedded => {
            if ctx.server_downloaded {
                commands::remove_server_download(&dispatch, vec![id]);
            } else {
                commands::download_on_server(&dispatch, vec![id]);
            }
        }
        SwipeAction::DownloadToDevice => commands::download_to_device(&dispatch, vec![id]),
        SwipeAction::RedownloadDevice => commands::redownload_device(&dispatch, vec![id]),
        SwipeAction::RemoveDownload => commands::remove_download(&dispatch, vec![id]),
        SwipeAction::ToggleDownload => {
            if ctx.downloaded_on_device {
                commands::remove_download(&dispatch, vec![id]);
            } else {
                commands::download_to_device(&dispatch, vec![id]);
            }
        }
        SwipeAction::DownloadOnServer => commands::download_on_server(&dispatch, vec![id]),
        SwipeAction::RemoveFromServer => commands::remove_server_download(&dispatch, vec![id]),
        SwipeAction::ToggleServerDownload => {
            if ctx.server_downloaded {
                commands::remove_server_download(&dispatch, vec![id]);
            } else {
                commands::download_on_server(&dispatch, vec![id]);
            }
        }
    }
    true
}
