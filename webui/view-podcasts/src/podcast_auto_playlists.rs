//! Replace the selected auto-playlist set online or queue it offline. The list pages/searches lazily, but submission
//! uses the full selected-ID set so offscreen choices survive; missing selected playlists load by ID.

use std::collections::HashSet;

use dioxus::prelude::*;

use crate::components::{FormPage, FormSubmit, PlaylistMultiselect};
use halogen_webui_commands::actions as commands;
use halogen_webui_component_icons::MagnifyingGlass;
use halogen_webui_hooks::{
    FormState, use_config, use_connection, use_dispatch, use_form_state, use_playlists,
    use_podcasts,
};
use halogen_webui_logging::warn;

/// Configure a podcast's auto-add playlists (`/podcasts/:id/auto-playlists`).
#[component]
pub fn PodcastAutoPlaylists(id: i32) -> Element {
    let playlists = use_playlists();
    let podcasts = use_podcasts();
    let connection = use_connection();
    let config = use_config();
    let dispatch = use_dispatch();
    let nav = use_navigator();

    let is_offline = connection.read().is_offline();

    // Selected playlist ids — seeded from the cached set, refreshed from the server
    // on mount when online. A `HashSet` for O(1) toggles.
    let mut selected = use_signal(|| {
        podcasts
            .read()
            .auto_playlists(id)
            .into_iter()
            .collect::<HashSet<i32>>()
    });
    // Insert-position override for this podcast's auto-added episodes:
    // `Some(true)` = start, `Some(false)` = end, `None` = server default. Seeded
    // and refreshed alongside `selected`.
    let mut add_to_start = use_signal(|| podcasts.read().auto_playlist_add_to_start(id));

    // Set-based form: no per-field validation, so `submitted` is unused.
    let form = use_form_state();
    let FormState {
        submitting,
        mut server_errors,
        ..
    } = form;
    let mut initialized = use_signal(|| false);
    let mut load_failed = use_signal(|| false);
    // In-flight guards: both resolve effects read reactive state, so without these
    // a publish mid-fetch would spawn overlapping requests. `fetching` gates the
    // one-shot membership resolve; `fetching_missing` dedupes per-id pool backfill.
    let mut fetching = use_signal(|| false);
    let mut fetching_missing = use_signal(HashSet::<i32>::new);

    // Search term for the lazy playlist multiselect (the picker owns its own pool).
    let mut search = use_signal(String::new);

    // Resolve the current auto-add set before letting the user edit it. `initialized`
    // is set ONLY once we have a trustworthy set — never on a failed/offline miss —
    // because submit PUTs the WHOLE set, so editing an unconfirmed (empty) selection
    // would wipe the server's real selections.
    use_effect(move || {
        if initialized() || *fetching.peek() {
            return;
        }
        if podcasts.read().auto_playlists_by_podcast.contains_key(&id) {
            selected.set(podcasts.read().auto_playlists(id).into_iter().collect());
            add_to_start.set(podcasts.read().auto_playlist_add_to_start(id));
            initialized.set(true);
            return;
        }
        // Read reactively (not `peek`) so a reconnect re-runs this and retries —
        // otherwise a failed offline deep-link resolve stays stuck on the alert until
        // some unrelated PodcastState publish.
        let offline = connection.read().is_offline();
        if offline {
            load_failed.set(true);
            return;
        }
        fetching.set(true);
        let cfg = config.peek().clone();
        spawn(async move {
            let Some(client) = cfg.api_client() else {
                load_failed.set(true);
                fetching.set(false);
                return;
            };
            load_failed.set(false);
            match client.get_podcast_auto_playlists(id).await {
                Ok(rows) => {
                    let ids: Vec<i32> = rows.iter().map(|r| r.playlist_id).collect();
                    // Every link carries the same per-podcast override (the set
                    // endpoint stamps it uniformly), so the first row speaks
                    // for the set.
                    let position = rows.first().and_then(|r| r.add_to_start);
                    commands::cache_auto_playlists(&dispatch, id, ids.clone(), position);
                    selected.set(ids.into_iter().collect());
                    add_to_start.set(position);
                    initialized.set(true);
                }
                Err(e) => {
                    warn!(podcast_id = id, error = %e, "Auto-playlists fetch failed");
                    load_failed.set(true);
                }
            }
            fetching.set(false);
        });
    });

    // Ensure currently-selected playlists are in the pool so their rows render
    // checked even when they're not on the loaded page. Fetches the missing ones by
    // id; settles once they're cached (pool then contains them).
    use_effect(move || {
        if !initialized() {
            return;
        }
        let pool: HashSet<i32> = playlists.peek().playlists.iter().map(|p| p.id).collect();
        let in_flight = fetching_missing.peek().clone();
        // Skip ids already in the pool OR already being fetched, so a selection
        // change mid-fetch can't re-request the same playlist.
        let missing: Vec<i32> = selected
            .read()
            .iter()
            .copied()
            .filter(|pid| !pool.contains(pid) && !in_flight.contains(pid))
            .collect();
        if missing.is_empty() {
            return;
        }
        fetching_missing.write().extend(missing.iter().copied());
        let cfg = config.peek().clone();
        spawn(async move {
            let Some(client) = cfg.api_client() else {
                // Release the reservations so a later run can retry.
                for pid in &missing {
                    fetching_missing.write().remove(pid);
                }
                return;
            };
            for pid in missing {
                if let Ok(pl) = client.get_playlist(pid).await {
                    commands::cache_playlists(&dispatch, vec![pl]);
                }
                fetching_missing.write().remove(&pid);
            }
        });
    });

    let server = server_errors();
    let catch_all: Vec<String> = server
        .as_ref()
        .map(|s| s.catch_all(&[]))
        .unwrap_or_default();
    let submit_disabled = submitting() || !initialized();

    let on_submit = move |evt: FormEvent| {
        evt.prevent_default();
        server_errors.set(None);
        // Build the payload from the SELECTED set directly — never from the rendered
        // rows (a partial pool would silently drop selected-but-unrendered ids).
        let ids: Vec<i32> = selected.read().iter().copied().collect();
        let position = *add_to_start.read();

        if is_offline {
            commands::set_podcast_auto_playlists(&dispatch, id, ids, position);
            nav.replace(format!("/podcasts/{id}", id = id));
            return;
        }

        form.spawn_submit(
            config,
            move |client| async move { client.set_podcast_auto_playlists(id, ids, position).await },
            move |rows| async move {
                let saved: Vec<i32> = rows.iter().map(|r| r.playlist_id).collect();
                commands::cache_auto_playlists(&dispatch, id, saved, position);
                nav.replace(format!("/podcasts/{id}", id = id));
            },
        );
    };

    let has_playlists = !playlists.read().playlists.is_empty();

    rsx! {
        FormPage { scroll_id: "auto-playlists-scroll",
            h1 { class: "text-2xl font-bold mb-1", "Auto-playlists" }
            p { class: "text-sm text-muted mb-4",
                "Pick the playlists new episodes of this podcast are automatically added to as they're found."
            }

            if !initialized() {
                if load_failed() {
                    div { role: "alert", class: "alert alert-error",
                        span { class: "text-sm",
                            if is_offline {
                                "You're offline — reconnect to edit this podcast's auto-playlists."
                            } else {
                                "Couldn't load the current auto-playlists. Check your connection and try again."
                            }
                        }
                    }
                } else {
                    div { class: "flex items-center gap-2 text-muted",
                        span { class: "loading loading-spinner loading-sm" }
                        "Loading auto-playlists…"
                    }
                }
            } else {
                // The picker is always mounted once membership resolves, so its
                // pool fetches even from a cold store; the search + Save chrome
                // appear once there are playlists to show (the picker itself shows
                // the "no playlists yet → Create" empty state when there are none).
                form { class: "space-y-4", onsubmit: on_submit,
                    if has_playlists {
                        label { class: "input input-bordered flex items-center gap-2",
                            MagnifyingGlass { class: "w-4 h-4 opacity-60" }
                            input {
                                r#type: "text",
                                class: "grow",
                                placeholder: "Search playlists…",
                                value: "{search}",
                                oninput: move |e| search.set(e.value()),
                            }
                        }
                    }

                    PlaylistMultiselect {
                        scroll_id: "auto-playlists-scroll",
                        sentinel_id: "auto-playlists-sentinel",
                        selected,
                        search,
                        on_change: Some(Callback::new(move |_| server_errors.set(None))),
                    }

                    if has_playlists {
                        label { class: "form-control",
                            div { class: "label",
                                span { class: "label-text", "New episodes are added to" }
                            }
                            select {
                                class: "select select-bordered",
                                value: match *add_to_start.read() {
                                    None => "default",
                                    Some(true) => "start",
                                    Some(false) => "end",
                                },
                                onchange: move |e| {
                                    add_to_start.set(match e.value().as_str() {
                                        "start" => Some(true),
                                        "end" => Some(false),
                                        _ => None,
                                    });
                                    server_errors.set(None);
                                },
                                option { value: "default", "Server default" }
                                option { value: "start", "Start of playlist" }
                                option { value: "end", "End of playlist" }
                            }
                        }
                    }

                    if has_playlists {
                        FormSubmit {
                            label: "Save",
                            submitting: submitting(),
                            disabled: submit_disabled,
                            offline: is_offline,
                            offline_hint: "You're offline — changes will sync when you reconnect.",
                            errors: catch_all,
                        }
                    }
                }
            }
        }
    }
}
