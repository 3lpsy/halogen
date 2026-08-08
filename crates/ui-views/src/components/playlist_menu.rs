//! Shared "playlist actions" context-menu builder.
//!
//! The playlist detail-header kebab and the playlist list-item kebab open the same
//! [`QuickMenu`](crate::components::QuickMenu) with the same actions, so the two
//! call sites stay in lock-step (one place to add/relabel an action). Mirrors
//! [`podcast_menu_sections`](crate::components::podcast_menu_sections): `on_delete`
//! is supplied by each caller so it can drive its own confirmation modal (the
//! delete is destructive — it removes the playlist server-side, for every device).

use std::rc::Rc;

// `Navigator` (the type) isn't re-exported through `dioxus::prelude` (only the
// `use_navigator` hook is), so reach it via the router facade directly.
use dioxus::router::Navigator;

use crate::Route;
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
                nav.push(Route::PlaylistReorderBy { id });
            }),
            QuickAction::new("Configure", QuickIcon::Settings, move || {
                nav.push(Route::PlaylistEdit { id });
            }),
        ],
        // Destructive — its own section so a divider sets it apart (mirrors the
        // podcast menu's Delete).
        vec![QuickAction::new("Delete", QuickIcon::Trash, move || {
            on_delete()
        })],
    ]
}
