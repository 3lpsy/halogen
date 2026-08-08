//! The episode-action taxonomy — one place for the user-visible label + icon of
//! each conceptual queue / server-download / device-download / playlist action.
//!
//! Both menu builders consume these (both now in `ui-episode-list`): the per-row
//! `episode_menu_sections` and the bulk `bulk_menu_sections`. Single vs bulk
//! *dispatch* stays in each builder; only the label/icon (the descriptor) lives
//! here, so the two can't drift apart (e.g. "Remove from Device" vs "Remove from
//! device", "Re-download on server" vs "Redownload on server").
//!
//! Label convention (N4): "<verb> <to/on/from> <target>" — "Download to device",
//! "Redownload on device", "Remove from device"; "Download on server",
//! "Redownload on server", "Remove from server"; "Add to queue" /
//! "Remove from queue"; "Add to playlist" / "Remove from playlist".

use crate::{QuickAction, QuickIcon};

/// A menu action's user-visible descriptor: the label string and the glyph. The
/// `on_select` closure (and `disabled`) is supplied by each builder, since single
/// and bulk dispatch differ.
#[derive(Clone, Copy)]
pub struct MenuAction {
    pub label: &'static str,
    pub icon: QuickIcon,
}

impl MenuAction {
    /// Bind this descriptor's label + icon to an `on_select` closure, producing an
    /// enabled [`QuickAction`]. The one place the taxonomy becomes a live menu row.
    /// Scope-independence rules are [`QuickAction::new`]'s.
    pub fn action(self, on_select: impl Fn() + 'static) -> QuickAction {
        QuickAction::new(self.label, self.icon, on_select)
    }
}

// Queue.
pub const ADD_TO_QUEUE: MenuAction = MenuAction {
    label: "Add to queue",
    icon: QuickIcon::Queue,
};
pub const REMOVE_FROM_QUEUE: MenuAction = MenuAction {
    label: "Remove from queue",
    icon: QuickIcon::Trash,
};

// Playlist.
pub const ADD_TO_PLAYLIST: MenuAction = MenuAction {
    label: "Add to playlist",
    icon: QuickIcon::AddPlaylist,
};
pub const REMOVE_FROM_PLAYLIST: MenuAction = MenuAction {
    label: "Remove from playlist",
    icon: QuickIcon::Trash,
};

// Server download.
pub const DOWNLOAD_ON_SERVER: MenuAction = MenuAction {
    label: "Download on server",
    icon: QuickIcon::CloudDownload,
};
pub const REDOWNLOAD_ON_SERVER: MenuAction = MenuAction {
    label: "Redownload on server",
    icon: QuickIcon::CloudDownload,
};
pub const REMOVE_FROM_SERVER: MenuAction = MenuAction {
    label: "Remove from server",
    icon: QuickIcon::Trash,
};

// Embedded-server download. The server is this device, so the only download
// concept left drops its qualifier — and the cloud glyph: nothing is remote.
pub const DOWNLOAD_EMBEDDED: MenuAction = MenuAction {
    label: "Download",
    icon: QuickIcon::Download,
};
pub const REDOWNLOAD_EMBEDDED: MenuAction = MenuAction {
    label: "Redownload",
    icon: QuickIcon::Download,
};
pub const REMOVE_DOWNLOAD_EMBEDDED: MenuAction = MenuAction {
    label: "Remove download",
    icon: QuickIcon::Trash,
};

// Device download.
pub const DOWNLOAD_TO_DEVICE: MenuAction = MenuAction {
    label: "Download to device",
    icon: QuickIcon::Download,
};
pub const REDOWNLOAD_ON_DEVICE: MenuAction = MenuAction {
    label: "Redownload on device",
    icon: QuickIcon::Download,
};
pub const REMOVE_FROM_DEVICE: MenuAction = MenuAction {
    label: "Remove from device",
    icon: QuickIcon::Trash,
};
