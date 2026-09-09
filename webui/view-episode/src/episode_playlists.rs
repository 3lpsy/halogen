//! Select episode playlist membership, including the queue, from a lazily paged pool. Fetch existing memberships by
//! endpoint, or use cached membership offline. Submit reconciles additions/removals through optimistic commands and
//! durable queuing; missing selected playlists are cached so they render checked.

use std::collections::HashSet;

use dioxus::prelude::*;
use halogen_wire::{DefaultListParams, PlaylistInclude};

use crate::components::PlaylistPickerScaffold;
use halogen_webui_commands::actions as commands;
use halogen_webui_hooks::{use_config, use_connection, use_dispatch, use_playlists};
use halogen_webui_logging::warn;

/// Membership the episode is already in, derived from the cached pool (the live
/// `episodes_by_playlist` index, falling back to each playlist's `episode_ids`).
fn cached_membership(
    playlists: &Signal<halogen_webui_app_state::PlaylistState>,
    episode_id: i32,
) -> HashSet<i32> {
    let st = playlists.peek();
    st.playlists
        .iter()
        .filter(|p| {
            st.episodes_by_playlist
                .get(&p.id)
                .map(|ids| ids.contains(&episode_id))
                .unwrap_or_else(|| {
                    p.episode_ids
                        .as_ref()
                        .is_some_and(|ids| ids.contains(&episode_id))
                })
        })
        .map(|p| p.id)
        .collect()
}

/// Multiselect playlist picker for one episode (`/episodes/:episode_id/playlists`).
#[component]
pub fn EpisodePlaylists(episode_id: i32) -> Element {
    let playlists = use_playlists();
    let connection = use_connection();
    let dispatch = use_dispatch();
    let config = use_config();
    let nav = use_navigator();

    // Selection + its starting snapshot (to diff on submit). Seeded once membership
    // resolves (online: authoritative endpoint; offline: cached index).
    let mut selected = use_signal(HashSet::<i32>::new);
    let mut initial = use_signal(HashSet::<i32>::new);
    let mut initialized = use_signal(|| false);
    // In-flight guard: the effect reads `app_state` (so it re-runs on every
    // publish), and `initialized` is only set when the async resolve finishes —
    // without this, a publish mid-fetch would spawn a second `list_episode_playlists`
    // request (last-write-wins on `selected`). `peek` so it doesn't re-trigger.
    let mut fetching = use_signal(|| false);

    // Resolve membership once. Online: the authoritative endpoint also fetches the
    // member playlists so they render checked even off the first page. Offline:
    // seed from the cached index so the user can still act.
    use_effect(move || {
        if initialized() || *fetching.peek() {
            return;
        }
        let offline = connection.read().is_offline();
        if offline {
            let seed = cached_membership(&playlists, episode_id);
            // Don't clobber a selection the user made while this was resolving (pre-init the only possible action is
            // checking boxes, so a non-empty `selected` means the user touched it), but MERGE the seed in rather than
            // skip it: dropping the real memberships would make the on-save diff emit removals for playlists the user
            // never unchecked.
            if selected.peek().is_empty() {
                selected.set(seed.clone());
            } else {
                selected.write().extend(seed.iter().copied());
            }
            initial.set(seed);
            initialized.set(true);
            return;
        }
        fetching.set(true);
        let cfg = config.peek().clone();
        spawn(async move {
            // One seed value from whichever source resolves; the cached index is the
            // fallback for "no client" and a failed fetch.
            let seed = match cfg.api_client() {
                None => cached_membership(&playlists, episode_id),
                Some(client) => {
                    let params = DefaultListParams::<PlaylistInclude> {
                        includes: Some(vec![PlaylistInclude::EpisodeIds]),
                        ..Default::default()
                    };
                    match client.list_episode_playlists(episode_id, params).await {
                        Ok(resp) => {
                            let ids: HashSet<i32> = resp.data.iter().map(|p| p.id).collect();
                            // Augment the pool + store so member rows render checked.
                            commands::cache_playlists(&dispatch, resp.data);
                            ids
                        }
                        Err(e) => {
                            warn!(episode_id, error = %e, "Episode membership fetch failed");
                            cached_membership(&playlists, episode_id)
                        }
                    }
                }
            };
            // Don't clobber a selection the user made while the fetch was in flight —
            // merge the seed in instead (see the offline branch above): keeping only
            // the user's mid-fetch picks would diff the untouched memberships as
            // removals on save. `initial` is the seed either way.
            initial.set(seed.clone());
            if selected.peek().is_empty() {
                selected.set(seed);
            } else {
                selected.write().extend(seed);
            }
            initialized.set(true);
            fetching.set(false);
        });
    });

    // Submit: reconcile the selection against the starting set and leave.
    let on_save = move |_| {
        let sel = selected.read().clone();
        let init = initial.read().clone();
        for pid in sel.difference(&init) {
            commands::add_to_playlist(&dispatch, *pid, vec![episode_id]);
        }
        for pid in init.difference(&sel) {
            commands::remove_from_playlist(&dispatch, *pid, vec![episode_id]);
        }
        nav.go_back();
    };

    let dirty = *selected.read() != *initial.read();

    rsx! {
        PlaylistPickerScaffold {
            title: "Add to playlist",
            scroll_id: "playlist-picker-scroll",
            sentinel_id: "playlist-picker-sentinel",
            selected,
            save_label: "Save",
            save_disabled: !dirty || !initialized(),
            on_save,
        }
    }
}
