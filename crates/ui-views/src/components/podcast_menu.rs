//! Shared "podcast actions" context-menu builder.
//!
//! The podcast detail header kebab and the podcast list-item kebab open the same
//! [`QuickMenu`](crate::components::QuickMenu) with the same actions — view
//! metadata, edit the podcast, edit/create the polling config, configure
//! auto-playlists, and delete. This keeps the two
//! call sites in lock-step (one place to add/relabel an action). `on_delete` is
//! supplied by each caller so it can drive its own confirmation modal (the delete
//! is destructive — it removes the podcast and all its episodes/downloads/history).

use std::rc::Rc;

// `Navigator` (the type) isn't re-exported through `dioxus::prelude` (only the
// `use_navigator` hook is), so reach it via the router facade directly.
use dioxus::router::Navigator;

use crate::Route;
use crate::components::{QuickAction, QuickIcon};

/// Build the podcast-actions menu sections for `id`.
///
/// `config_id` is the podcast's `podcast_config_id` FK: `Some` → "Edit polling
/// config", `None` → "Create polling config". `nav` navigates to the config /
/// auto-playlist routes. `on_remove_local_data` fires the caller's local-prune
/// confirm flow (recovery, server untouched); `on_delete` fires the caller's
/// delete-confirm flow (unsubscribe, server-side). Both are scope-independent
/// `Rc` closures — see `QuickAction` for why (the menu can outlive the caller).
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
                nav.push(Route::PodcastConfigEdit { id, config_id });
            })
        }
        None => QuickAction::new("Create polling config", QuickIcon::Settings, move || {
            nav.push(Route::PodcastConfigCreate { id });
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
                nav.push(Route::PodcastMetadata { id });
            },
        )],
        vec![
            QuickAction::new("Edit podcast", QuickIcon::Edit, move || {
                nav.push(Route::PodcastEdit { id });
            }),
            config_action,
            QuickAction::new("Configure auto-playlists", QuickIcon::Queue, move || {
                nav.push(Route::PodcastAutoPlaylists { id });
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
