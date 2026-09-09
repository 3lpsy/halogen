//! Share playlist actions between list and detail menus. Callers supply confirmation for server-side deletion, which
//! affects every device.

use std::rc::Rc;

// `Navigator` (the type) isn't re-exported through `dioxus::prelude` (only the
// `use_navigator` hook is), so reach it via the router facade directly.
use dioxus::router::Navigator;

use crate::components::{QuickAction, QuickIcon};

/// Build the playlist-actions menu sections for `id`. `nav` navigates to the
/// playlist's action routes; `on_delete` fires the caller's delete-confirm flow
/// (a scope-independent `Rc` closure — see `QuickAction` for why: the menu can
/// outlive the caller).
pub fn playlist_menu_sections(
    id: i32,
    nav: Navigator,
    on_delete: Rc<dyn Fn()>,
) -> Vec<Vec<QuickAction>> {
    vec![
        vec![
            QuickAction::new("Reorder", QuickIcon::Reorder, move || {
                nav.push(format!("/playlists/{id}/reorder-by", id = id));
            }),
            QuickAction::new("Configure", QuickIcon::Settings, move || {
                nav.push(format!("/playlists/{id}/edit", id = id));
            }),
        ],
        // Destructive — its own section so a divider sets it apart (mirrors the
        // podcast menu's Delete).
        vec![QuickAction::new("Delete", QuickIcon::Trash, move || {
            on_delete()
        })],
    ]
}
