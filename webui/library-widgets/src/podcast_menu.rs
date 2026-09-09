//! Build identical podcast actions for list and detail menus. Callers supply deletion confirmation because unsubscribe
//! removes server-side episodes, downloads, and history.

use std::rc::Rc;

// `Navigator` (the type) isn't re-exported through `dioxus::prelude` (only the
// `use_navigator` hook is), so reach it via the router facade directly.
use dioxus::router::Navigator;

use crate::components::{QuickAction, QuickIcon};

/// Build actions for id; config_id selects create versus edit polling config. Local-prune and server-delete callbacks
/// invoke separate confirmations. Use scope-independent Rc callbacks because the menu may outlive its caller.
pub fn podcast_menu_sections(
    id: i32,
    config_id: Option<i32>,
    nav: Navigator,
    on_remove_local_data: Rc<dyn Fn()>,
    on_delete: Rc<dyn Fn()>,
) -> Vec<Vec<QuickAction>> {
    let config_action = match config_id {
        Some(config_id) => {
            QuickAction::new("Edit polling config", QuickIcon::Settings, move || {
                nav.push(format!(
                    "/podcasts/{id}/config/{config_id}/edit",
                    id = id,
                    config_id = config_id
                ));
            })
        }
        None => QuickAction::new("Create polling config", QuickIcon::Settings, move || {
            nav.push(format!("/podcasts/{id}/config/create", id = id));
        }),
    };

    vec![
        // Metadata first: the FIRST section renders at the TOP of the
        // bottom-anchored mobile panel (farthest from the thumb — it's the
        // least-used, read-only action).
        vec![QuickAction::new(
            "View metadata",
            QuickIcon::Metadata,
            move || {
                nav.push(format!("/podcasts/{id}/metadata", id = id));
            },
        )],
        vec![
            QuickAction::new("Edit podcast", QuickIcon::Edit, move || {
                nav.push(format!("/podcasts/{id}/edit", id = id));
            }),
            config_action,
            QuickAction::new("Configure auto-playlists", QuickIcon::Queue, move || {
                nav.push(format!("/podcasts/{id}/auto-playlists", id = id));
            }),
        ],
        // Local recovery — above Delete so the most-destructive (server-side)
        // action stays last. Wipes this podcast + its episodes' local cache only;
        // the subscription survives and re-syncs on the next pull.
        vec![QuickAction::new(
            "Remove local data",
            QuickIcon::Reset,
            move || on_remove_local_data(),
        )],
        // Destructive — its own section so a divider sets it apart.
        vec![QuickAction::new("Delete", QuickIcon::Trash, move || {
            on_delete()
        })],
    ]
}
