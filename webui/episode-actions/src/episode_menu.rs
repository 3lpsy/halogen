//! Share episode menus between list and detail views. Detail omits View Episode and playlist reorder; callers supply
//! local-data recovery confirmation, while the host drops empty sections.

use std::rc::Rc;

use dioxus::prelude::*;
// `Navigator` (the type) isn't re-exported through `dioxus::prelude` (only the
// `use_navigator` hook is), so reach it via the router facade directly.
use dioxus::router::Navigator;

use halogen_webui_commands::Command;
use halogen_webui_commands::actions as commands;
use halogen_webui_component_widgets::{
    ADD_TO_PLAYLIST, ADD_TO_QUEUE, DOWNLOAD_EMBEDDED, DOWNLOAD_ON_SERVER, DOWNLOAD_TO_DEVICE,
    QuickAction, QuickIcon, REDOWNLOAD_EMBEDDED, REDOWNLOAD_ON_SERVER, REMOVE_DOWNLOAD_EMBEDDED,
    REMOVE_FROM_DEVICE, REMOVE_FROM_QUEUE, REMOVE_FROM_SERVER,
};
use halogen_webui_config::PlaybackPreference;
use halogen_webui_player::PlayerController;

/// Live episode/download/queue state the caller has already resolved from
/// `EpisodeState`, plus the per-list reorder context. Built into menu sections by
/// [`episode_menu_sections`].
pub struct EpisodeMenuArgs {
    pub episode_id: i32,
    pub podcast_id: i32,
    /// Current playback preference (gates the "Stream from server" escape hatch).
    pub playback_pref: PlaybackPreference,
    /// Embedded-server mode: the device-download concept doesn't exist (Play IS
    /// streaming from this device's server), so the device action and the stream
    /// escape hatch are hidden and the server actions drop their qualifier.
    pub embedded: bool,
    pub server_downloaded: bool,
    pub server_downloading: bool,
    /// Live server-download percent (0–100) while `server_downloading`, for the
    /// progress label. `None` before the tracker reports / for unknown-length.
    pub server_download_progress: Option<u8>,
    pub downloaded_on_device: bool,
    pub device_downloading: bool,
    pub is_offline: bool,
    /// The queue (default playlist) id, if one exists — `None` hides the queue
    /// action entirely (nowhere to add).
    pub queue_id: Option<i32>,
    pub in_queue: bool,
    /// The playlist this row belongs to, when the list is a playlist/queue. `None`
    /// (the detail page, or a non-playlist list) yields an empty reorder section.
    pub playlist_id: Option<i32>,
    /// Manual (Custom) order is active → the reorder actions are enabled.
    pub reorder_enabled: bool,
    /// This row's index within the rendered list (reorder boundary math).
    pub position: usize,
    /// Total rendered rows. Not the Move Down / Move Last boundary — that uses
    /// `full_len` (see below) so a windowed playlist's bottom visible row isn't capped.
    pub list_len: usize,
    /// The list is shown in descending Custom order (down arrow) → reorder targets
    /// are mirrored back to stored ascending positions before dispatch.
    pub reversed: bool,
    /// Full (un-windowed) pool length — the axis used to mirror reorder targets
    /// when `reversed`.
    pub full_len: usize,
    /// Include the "View episode" nav action. False on the detail page itself.
    pub include_view_episode: bool,
}

/// Capture scope-independent handles because menus can outlive their rows. Order sections as metadata, downloads,
/// navigation, reorder, playlists, queue, recovery so frequent mobile actions stay near the bottom; the host removes
/// empty sections.
pub fn episode_menu_sections(
    args: EpisodeMenuArgs,
    player: Signal<PlayerController>,
    dispatch: Coroutine<Command>,
    nav: Navigator,
    on_remove_local_data: Rc<dyn Fn()>,
) -> Vec<Vec<QuickAction>> {
    let EpisodeMenuArgs {
        episode_id,
        podcast_id,
        playback_pref,
        embedded,
        server_downloaded,
        server_downloading,
        server_download_progress,
        downloaded_on_device,
        device_downloading,
        is_offline,
        queue_id,
        in_queue,
        playlist_id,
        reorder_enabled,
        position,
        // Not the Move Down/Last boundary — that's `full_len` (see below), so a
        // windowed playlist's bottom visible row isn't greyed while items remain
        // below it. Kept on the struct for the call sites that still pass it.
        list_len: _,
        reversed,
        full_len,
        include_view_episode,
    } = args;

    // Metadata first: the FIRST section renders at the TOP of the bottom-anchored
    // mobile panel (farthest from the thumb — the least-used, read-only action).
    // Path-string nav: this crate is route-free (see the crate docs).
    let metadata_section = vec![QuickAction::new(
        "View metadata",
        QuickIcon::Metadata,
        move || {
            nav.push(format!("/episodes/{episode_id}/metadata"));
        },
    )];

    // Downloads section. Top→bottom: stream escape hatch, server actions
    // (re-download topmost), then the device action closest to the divider.
    let mut downloads = Vec::new();
    // Explicit server streaming — the escape hatch from local-first playback.
    // Hidden under DownloadOnly ("never streams"), when offline, when there's
    // nothing on the server to stream, or when a device copy makes it moot.
    if !embedded
        && playback_pref != PlaybackPreference::DownloadOnly
        && server_downloaded
        && !is_offline
        && !downloaded_on_device
    {
        downloads.push(QuickAction::new(
            "Stream from server",
            QuickIcon::Play,
            move || {
                player().stream_episode_in(episode_id, playlist_id);
            },
        ));
    }
    if server_downloading {
        // Non-actionable status entry while a fetch is in flight; carries the live
        // percent once the server's tracker reports it. Embedded mode drops the
        // qualifier + cloud glyph (nothing is remote).
        let label = match (embedded, server_download_progress) {
            (true, Some(pct)) => format!("Downloading… {pct}%"),
            (true, None) => "Downloading…".to_string(),
            (false, Some(pct)) => format!("Downloading on server… {pct}%"),
            (false, None) => "Downloading on server…".to_string(),
        };
        let icon = if embedded {
            QuickIcon::Download
        } else {
            QuickIcon::CloudDownload
        };
        downloads.push(QuickAction::new(label, icon, move || {}));
    } else if server_downloaded {
        // Already on the server: offer a re-download and a removal.
        let (redownload, remove) = if embedded {
            (REDOWNLOAD_EMBEDDED, REMOVE_DOWNLOAD_EMBEDDED)
        } else {
            (REDOWNLOAD_ON_SERVER, REMOVE_FROM_SERVER)
        };
        downloads.push(redownload.action(move || {
            commands::redownload_on_server(&dispatch, vec![episode_id]);
        }));
        downloads.push(remove.action(move || {
            commands::remove_server_download(&dispatch, vec![episode_id]);
        }));
    } else {
        let download = if embedded {
            DOWNLOAD_EMBEDDED
        } else {
            DOWNLOAD_ON_SERVER
        };
        downloads.push(download.action(move || {
            commands::download_on_server(&dispatch, vec![episode_id]);
        }));
    }
    // Device action. Mirrors the badge: spinner-equivalent (non-actionable) while
    // the bytes are in flight, remove once stored. The transient "Downloading…"
    // status isn't part of the shared taxonomy (no static action behind it).
    // Absent entirely in embedded mode — the server download above IS the local copy.
    if !embedded {
        let device_action = if downloaded_on_device {
            REMOVE_FROM_DEVICE
        } else {
            DOWNLOAD_TO_DEVICE
        };
        let device_label = if device_downloading && !downloaded_on_device {
            "Downloading to device…".to_string()
        } else {
            device_action.label.to_string()
        };
        downloads.push(QuickAction::new(
            device_label,
            device_action.icon,
            move || {
                if downloaded_on_device {
                    commands::remove_download(&dispatch, vec![episode_id]);
                } else if !device_downloading {
                    commands::download_to_device(&dispatch, vec![episode_id]);
                }
            },
        ));
    }

    // Navigation: podcast always; episode unless we're already on its page.
    let mut navigation = vec![QuickAction::new(
        "View podcast",
        QuickIcon::Podcast,
        move || {
            nav.push(format!("/podcasts/{podcast_id}"));
        },
    )];
    if include_view_episode {
        navigation.push(QuickAction::new(
            "View episode",
            QuickIcon::Episode,
            move || {
                nav.push(format!("/episodes/{episode_id}"));
            },
        ));
    }

    // Reorder actions — only for playlist/queue rows. Greyed out (disabled) when
    // the list isn't in Custom order, or at the list boundary. Targets are *visual*
    // (displayed-list) indices: First/Last/Up/Down all read as the user sees them;
    // `move_in_playlist_visual` mirrors them to stored positions when descending.
    let at_start = position == 0;
    // Disable Move Down / Move Last only on the FULL pool's last row, not the
    // rendered window's: a paged playlist windows the TOP of a longer membership
    // (`take(vis)`), so the bottom *visible* row still has items below it. `full_len`
    // is the full pool count; `list_len` (the rendered window) would grey them early.
    let at_end = position + 1 >= full_len;
    let move_actions: Vec<QuickAction> = match playlist_id {
        Some(pid) => {
            let move_to = move |target: i32| {
                move || {
                    commands::move_in_playlist_visual(
                        &dispatch, pid, episode_id, target, reversed, full_len,
                    );
                }
            };
            vec![
                QuickAction::new("Move Up", QuickIcon::MoveUp, move_to(position as i32 - 1))
                    .disabled(!reorder_enabled || at_start),
                QuickAction::new(
                    "Move Down",
                    QuickIcon::MoveDown,
                    move_to(position as i32 + 1),
                )
                .disabled(!reorder_enabled || at_end),
                QuickAction::new("Move First", QuickIcon::MoveFirst, move_to(0))
                    .disabled(!reorder_enabled || at_start),
                QuickAction::new(
                    "Move Last",
                    QuickIcon::MoveLast,
                    move_to(full_len as i32 - 1),
                )
                .disabled(!reorder_enabled || at_end),
            ]
        }
        None => Vec::new(),
    };

    // Add-to-playlist — opens the multiselect picker page. Directly above "Add to
    // queue" on mobile (bottom-anchored) and directly below it on desktop.
    let add_to_playlist_section = vec![ADD_TO_PLAYLIST.action(move || {
        nav.push(format!("/episodes/{episode_id}/playlists"));
    })];

    // Queue action — only when a queue (default playlist) exists. Empty otherwise,
    // so the menu host filters it out (no "add to queue" with nowhere to add).
    let queue_section: Vec<QuickAction> = match queue_id {
        Some(qid) => {
            let descriptor = if in_queue {
                REMOVE_FROM_QUEUE
            } else {
                ADD_TO_QUEUE
            };
            vec![descriptor.action(move || {
                if in_queue {
                    commands::remove_from_playlist(&dispatch, qid, vec![episode_id]);
                } else {
                    commands::add_to_playlist(&dispatch, qid, vec![episode_id]);
                }
            })]
        }
        None => Vec::new(),
    };

    // Local recovery — its own divider-separated section. Wipes this episode's
    // local cache (audio, progress, metadata, queue membership) WITHOUT touching
    // the server; it re-syncs on the next pull. See `confirm_purge`.
    let recovery_section = vec![QuickAction::new(
        "Remove local data",
        QuickIcon::Reset,
        move || on_remove_local_data(),
    )];

    vec![
        metadata_section,
        downloads,
        navigation,
        move_actions,
        add_to_playlist_section,
        queue_section,
        recovery_section,
    ]
}
