//! Add many selected episodes to one or more playlists (bulk multiselect picker).
//!
//! Opened from the episode-list multiselect "Add to playlist" action, carrying the
//! selected episode ids comma-joined in the path. Unlike the single-episode
//! [`EpisodePlaylists`](crate::pages::episode::episode_playlists::EpisodePlaylists) picker
//! this is **add-only**: no membership pre-selection and no remove/reconcile — pick
//! the target playlist(s) and Save bulk-adds every selected episode to each, through
//! the always-optimistic + outbox `bulk_add_to_playlist` binding.

use std::collections::HashSet;

use dioxus::prelude::*;

use crate::components::{BackButton, PlaylistPickerScaffold};
use halogen_ui_state::commands;
use halogen_ui_state::hooks::use_dispatch;

/// Parse the comma-joined episode ids carried in the route path.
fn parse_ids(raw: &str) -> Vec<i32> {
    raw.split(',')
        .filter_map(|s| s.trim().parse::<i32>().ok())
        .collect()
}

/// Bulk "add to playlist" picker (`/episodes/bulk/playlists/:ids`).
#[component]
pub fn BulkEpisodePlaylists(ids: String) -> Element {
    let dispatch = use_dispatch();
    let nav = use_navigator();

    let episode_ids = parse_ids(&ids);
    let n = episode_ids.len();

    // Target playlists to add into. Add-only → starts empty, no membership seed.
    let selected = use_signal(HashSet::<i32>::new);

    // Submit: bulk-add all carried episodes to each chosen playlist, then leave.
    let on_save = {
        let episode_ids = episode_ids.clone();
        move |_| {
            for pid in selected.read().iter().copied() {
                commands::add_to_playlist(&dispatch, pid, episode_ids.clone());
            }
            nav.go_back();
        }
    };

    let any_selected = !selected.read().is_empty();

    // An empty/garbage id list (bad deep link) would otherwise render a working
    // "Add 0 to playlist" picker whose Save is a no-op — show a clear dead-end
    // instead. (After all hooks, so the rules of hooks still hold.)
    if episode_ids.is_empty() {
        return rsx! {
            div { class: "p-4 flex flex-col items-start gap-3",
                BackButton {}
                p { class: "text-muted", "No episodes selected." }
            }
        };
    }

    rsx! {
        PlaylistPickerScaffold {
            title: format!("Add {n} to playlist"),
            scroll_id: "bulk-playlist-picker-scroll",
            sentinel_id: "bulk-playlist-picker-sentinel",
            selected,
            save_label: "Add to selected",
            save_disabled: !any_selected,
            on_save,
        }
    }
}
