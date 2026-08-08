//! Bulk-action menu builder for the episode list's multiselect mode.
//!
//! Pure data construction (no reactive reads): given the selected ids plus the
//! resolved queue / current-playlist context, builds the [`QuickAction`] sections
//! the shared [`QuickContextMenu`](halogen_ui_widgets::QuickMenu) renders. Each
//! action dispatches the matching bulk worker command (or, for "Add to playlist",
//! navigates to the bulk picker) for a snapshot of the ids.

use dioxus::prelude::*;
// `Navigator` (the type) isn't re-exported through `dioxus::prelude` (only the
// `use_navigator` hook is), so reach it via the router facade directly.
use dioxus::router::Navigator;

use halogen_ui_commands::Command;
use halogen_ui_state::commands;
use halogen_ui_widgets::{
    ADD_TO_PLAYLIST, ADD_TO_QUEUE, DOWNLOAD_EMBEDDED, DOWNLOAD_ON_SERVER, DOWNLOAD_TO_DEVICE,
    MenuAction, QuickAction, REDOWNLOAD_EMBEDDED, REDOWNLOAD_ON_DEVICE, REDOWNLOAD_ON_SERVER,
    REMOVE_DOWNLOAD_EMBEDDED, REMOVE_FROM_DEVICE, REMOVE_FROM_PLAYLIST, REMOVE_FROM_QUEUE,
    REMOVE_FROM_SERVER,
};

/// Build the bulk-action menu sections for a multiselect over `ids`.
///
/// `queue_id` is the default playlist (queue) id when one exists; `current_playlist_id`
/// is the playlist the list is showing (when it is a playlist/queue view) — it gates
/// "Remove from playlist". `nav` is used by "Add to playlist" to open the bulk picker.
pub(super) fn bulk_menu_sections(
    ids: Vec<i32>,
    dispatch: Coroutine<Command>,
    nav: Navigator,
    queue_id: Option<i32>,
    current_playlist_id: Option<i32>,
    embedded: bool,
) -> Vec<Vec<QuickAction>> {
    // Queue add/remove — only when a queue (default playlist) exists. The bulk
    // endpoints are lenient/idempotent, so we always offer both (already-queued ids
    // are skipped on add, non-members on remove). Empty section is filtered by the host.
    // Each bulk action snapshots a fresh clone of `ids` for its closure.
    let bind = |descriptor: MenuAction, f: fn(&Coroutine<Command>, Vec<i32>)| -> QuickAction {
        let ids = ids.clone();
        descriptor.action(move || f(&dispatch, ids.clone()))
    };

    let queue_section: Vec<QuickAction> = match queue_id {
        Some(qid) => vec![
            ADD_TO_QUEUE.action({
                let ids = ids.clone();
                move || commands::add_to_playlist(&dispatch, qid, ids.clone())
            }),
            REMOVE_FROM_QUEUE.action({
                let ids = ids.clone();
                move || commands::remove_from_playlist(&dispatch, qid, ids.clone())
            }),
        ],
        None => Vec::new(),
    };

    // Playlist section: "Add to playlist" always (opens the bulk picker carrying the
    // ids); "Remove from playlist" only on a playlist view that isn't the queue (the
    // queue section already covers queue removal).
    let mut playlist_section = vec![ADD_TO_PLAYLIST.action({
        let ids = ids.clone();
        move || {
            let joined = ids.iter().map(i32::to_string).collect::<Vec<_>>().join(",");
            nav.push(format!("/episodes/bulk/playlists/{joined}"));
        }
    })];
    if let Some(pid) = current_playlist_id
        && Some(pid) != queue_id
    {
        playlist_section.push(REMOVE_FROM_PLAYLIST.action({
            let ids = ids.clone();
            move || commands::remove_from_playlist(&dispatch, pid, ids.clone())
        }));
    }

    // Embedded mode: only the server-download concept exists — its section takes
    // the unqualified labels and the device section vanishes (empty sections are
    // filtered by the host).
    let server_section = if embedded {
        vec![
            bind(REDOWNLOAD_EMBEDDED, commands::redownload_on_server),
            bind(DOWNLOAD_EMBEDDED, commands::download_on_server),
            bind(REMOVE_DOWNLOAD_EMBEDDED, commands::remove_server_download),
        ]
    } else {
        vec![
            bind(REDOWNLOAD_ON_SERVER, commands::redownload_on_server),
            bind(DOWNLOAD_ON_SERVER, commands::download_on_server),
            bind(REMOVE_FROM_SERVER, commands::remove_server_download),
        ]
    };
    let device_section = if embedded {
        Vec::new()
    } else {
        vec![
            bind(REDOWNLOAD_ON_DEVICE, commands::redownload_device),
            bind(DOWNLOAD_TO_DEVICE, commands::download_to_device),
            bind(REMOVE_FROM_DEVICE, commands::remove_download),
        ]
    };
    vec![
        queue_section,
        playlist_section,
        server_section,
        device_section,
    ]
}
