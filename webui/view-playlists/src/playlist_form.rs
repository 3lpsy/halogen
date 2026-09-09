//! Validate playlist data locally and merge server field/banner errors into FormErrors. Create is online-only for its
//! new ID; edit can queue optimistic offline changes. If no default queue exists, create forces make-default, also
//! enforced server-side.

use dioxus::prelude::*;
use halogen_wire::{PlaylistStoreData, PlaylistUpdateData, Validate};

use crate::components::{FormErrors, FormPage, FormSubmit, InputField, ToggleField};
use halogen_webui_app_state::QueueState;
use halogen_webui_commands::actions as commands;
use halogen_webui_hooks::{
    FormState, use_config, use_connection, use_deep_link_resource, use_dispatch, use_form_state,
    use_playlists,
};

/// Which flow this form drives.
#[derive(Clone, Copy, PartialEq)]
pub enum FormMode {
    Create,
    Edit { id: i32 },
}

/// Create a new playlist (`/playlists/create`).
#[component]
pub fn PlaylistCreate() -> Element {
    rsx! { PlaylistForm { mode: FormMode::Create } }
}

/// Edit an existing playlist (`/playlists/:id/edit`).
#[component]
pub fn PlaylistEdit(id: i32) -> Element {
    rsx! { PlaylistForm { mode: FormMode::Edit { id } } }
}

/// Creation may omit a blank description; edits must send it to clear the saved value.
fn description_field(is_create: bool, value: &str) -> Option<String> {
    let value = value.trim();
    (!is_create || !value.is_empty()).then(|| value.to_string())
}

/// Validate the live form values against the typed `*Data` for the flow. The empty
/// result means valid. Both flows share the `name` rule (`length(min = 1)`).
fn validate_fields(is_create: bool, name: &str, description: &str, is_default: bool) -> FormErrors {
    let result = if is_create {
        PlaylistStoreData {
            name: name.to_string(),
            description: description_field(is_create, description),
            is_default: Some(is_default),
            ..Default::default()
        }
        .validate()
    } else {
        PlaylistUpdateData {
            name: Some(name.to_string()),
            description: description_field(is_create, description),
            is_default: Some(is_default),
            ..Default::default()
        }
        .validate()
    };
    result
        .err()
        .map(|e| FormErrors::from_validation(&e))
        .unwrap_or_default()
}

#[component]
fn PlaylistForm(mode: FormMode) -> Element {
    let playlists = use_playlists();
    let config = use_config();
    let dispatch = use_dispatch();
    let nav = use_navigator();

    let is_create = matches!(mode, FormMode::Create);
    let is_offline = use_connection().read().is_offline();
    // The queue is the default playlist and may not exist. While none exists, the
    // first playlist created MUST become it — so on create we lock "make default"
    // on (the server enforces this too). Resolve the queue once if still Unknown so
    // the lock is accurate (one-shot guard + `peek` so it can't loop on publishes).
    let mut resolve_requested = use_signal(|| false);
    use_effect(move || {
        if resolve_requested() {
            return;
        }
        resolve_requested.set(true);
        if matches!(playlists.peek().queue, QueueState::Unknown) {
            commands::ensure_default_playlist(&dispatch);
        }
    });
    let force_default = is_create && matches!(playlists.read().queue, QueueState::Absent);

    // Field values.
    let mut name = use_signal(String::new);
    let mut description = use_signal(String::new);
    let mut is_default = use_signal(|| false);
    let mut delete_server_file = use_signal(|| false);
    let mut delete_client_file = use_signal(|| false);

    // Local-error gating: a field's error shows once it's been touched (blurred) or
    // after a submit attempt — never nags an empty form before the user types.
    let name_touched = use_signal(|| false);

    // Shared async + server-error scaffolding (submitting / server_errors /
    // submitted). `FormState` is `Copy`, so we keep `form` (for `spawn_submit`)
    // while also destructuring into the same locals the form body uses.
    let form = use_form_state();
    let FormState {
        submitting,
        mut server_errors,
        mut submitted,
    } = form;

    // Edit deep-link: playlists aren't bulk-held, so a hard refresh / deep link to `/playlists/:id/edit` lands with a
    // COLD pool, the prefill below would find nothing, leaving the form blank, and a save would write defaults (notably
    // `is_default: false`, un-defaulting the queue) over the real playlist. Fetch + cache it so the prefill can
    // populate the form (mirrors `PlaylistDetail`). In Create mode `present` is always true, so the hook no-ops.
    let _ = use_deep_link_resource(
        config,
        move || match mode {
            FormMode::Create => true,
            FormMode::Edit { id } => playlists.read().playlists.iter().any(|p| p.id == id),
        },
        move |client| async move {
            if let FormMode::Edit { id } = mode {
                let pl = client.get_playlist(id).await.map_err(|e| e.to_string())?;
                commands::cache_playlists(&dispatch, vec![pl]);
            }
            Ok(())
        },
    );

    // Edit: prefill once the playlist lands in the pool. Guarded so a later EpisodeState
    // publish (or a keystroke-triggered re-render) never clobbers the user's edits.
    let mut initialized = use_signal(|| false);
    use_effect(move || {
        if initialized() {
            return;
        }
        match mode {
            FormMode::Create => initialized.set(true),
            FormMode::Edit { id } => {
                if let Some(pl) = playlists
                    .read()
                    .playlists
                    .iter()
                    .find(|p| p.id == id)
                    .cloned()
                {
                    name.set(pl.name);
                    description.set(pl.description.unwrap_or_default());
                    is_default.set(pl.is_default);
                    delete_server_file.set(pl.on_remove_delete_file_server);
                    delete_client_file.set(pl.on_remove_delete_file_client);
                    initialized.set(true);
                }
            }
        }
    });

    // When the queue is absent, force "make default" on for the create (the
    // checkbox is also disabled). Guarded read of `is_default` so it settles.
    use_effect(move || {
        if is_create && matches!(playlists.read().queue, QueueState::Absent) && !is_default() {
            is_default.set(true);
        }
    });

    // Live local validation → gates the submit button + drives inline field errors.
    let local_errors = validate_fields(is_create, &name(), &description(), is_default());
    let invalid = !local_errors.is_empty();

    // Inline messages: local (only once touched / submitted) plus any server field
    // error. Server errors are cleared on edit, so they never linger past a fix.
    let server = server_errors();
    let show_local_name = name_touched() || submitted();
    let name_messages = local_errors.field_messages("name", show_local_name, server.as_ref());
    let description_messages = local_errors.field_messages("description", false, server.as_ref());
    // Everything that isn't a form field → the catch-all banner below the button.
    let catch_all: Vec<String> = server
        .as_ref()
        .map(|s| s.catch_all(&["name", "description"]))
        .unwrap_or_default();

    // Create needs a real server id, so it's online-only; edit works offline via the outbox. So only create is disabled
    // offline. Edit also can't submit until the existing values have prefilled (`initialized`): on a cold-cache deep
    // link a save before the fetch lands would write form defaults (e.g. `is_default: false`) over the real playlist.
    let edit_unloaded = !is_create && !initialized();
    let submit_disabled = submitting() || invalid || (is_create && is_offline) || edit_unloaded;
    let submit_label = if is_create { "Create" } else { "Save changes" };
    let title = if is_create {
        "New playlist"
    } else {
        "Edit playlist"
    };

    let on_submit = move |evt: FormEvent| {
        evt.prevent_default();
        submitted.set(true);
        server_errors.set(None);
        // Never submit an edit before its values loaded (see `edit_unloaded`).
        if !is_create && !initialized() {
            return;
        }
        if !validate_fields(is_create, &name(), &description(), is_default()).is_empty() {
            return;
        }

        let name_val = name();
        let desc_val = description_field(is_create, &description());
        let default_val = is_default();
        let del_server_val = delete_server_file();
        let del_client_val = delete_client_file();

        // Offline EDIT: optimistic apply + outbox, then leave (no synchronous server
        // errors — local validation already passed). Offline CREATE can't happen
        // (the button is disabled). Online: direct API below, which shows errors.
        if is_offline {
            if let FormMode::Edit { id } = mode {
                commands::update_playlist(
                    &dispatch,
                    id,
                    PlaylistUpdateData {
                        name: Some(name_val),
                        description: desc_val,
                        is_default: Some(default_val),
                        on_remove_delete_file_server: Some(del_server_val),
                        on_remove_delete_file_client: Some(del_client_val),
                    },
                );
                nav.replace(format!("/playlists/{id}", id = id));
            }
            return;
        }

        form.spawn_submit(
            config,
            move |client| async move {
                match mode {
                    FormMode::Create => {
                        client
                            .create_playlist(PlaylistStoreData {
                                name: name_val,
                                description: desc_val,
                                is_default: Some(default_val),
                                on_remove_delete_file_server: Some(del_server_val),
                                on_remove_delete_file_client: Some(del_client_val),
                            })
                            .await
                    }
                    FormMode::Edit { id } => {
                        client
                            .update_playlist(
                                id,
                                PlaylistUpdateData {
                                    name: Some(name_val),
                                    description: desc_val,
                                    is_default: Some(default_val),
                                    on_remove_delete_file_server: Some(del_server_val),
                                    on_remove_delete_file_client: Some(del_client_val),
                                },
                            )
                            .await
                    }
                }
            },
            move |pl| async move {
                let id = pl.id;
                // Optimistically cache (instant nav), then reconcile via a pull.
                commands::cache_playlists(&dispatch, vec![pl]);
                commands::refresh(&dispatch);
                nav.replace(format!("/playlists/{id}", id = id));
            },
        );
    };

    rsx! {
        FormPage {
            h1 { class: "text-2xl font-bold mb-4", "{title}" }

            form { class: "space-y-4", onsubmit: on_submit,
                // Name (required).
                InputField {
                    label: "Name",
                    placeholder: "Playlist name",
                    autofocus: true,
                    value: name,
                    touched: Some(name_touched),
                    messages: name_messages,
                    on_input: move |()| server_errors.set(None),
                }

                // Description (optional).
                InputField {
                    label: "Description",
                    textarea: true,
                    placeholder: "Optional",
                    value: description,
                    messages: description_messages,
                    on_input: move |()| server_errors.set(None),
                }

                // Make default. Locked on when there's no queue yet — the first
                // playlist must become it.
                ToggleField {
                    label: "Make this the default queue",
                    checked: is_default(),
                    disabled: force_default,
                    hint: if force_default {
                        "You don't have a queue yet — this playlist will become it.".to_string()
                    } else {
                        String::new()
                    },
                    onchange: move |v| {
                        is_default.set(v);
                        server_errors.set(None);
                    },
                }

                // Delete-on-remove cleanup flags. The server file is shared (one
                // copy per episode), so the server only deletes it once the
                // episode belongs to no other playlist.
                ToggleField {
                    label: "Delete server download when an episode is removed",
                    checked: delete_server_file(),
                    hint: "Removing an episode from this playlist deletes its downloaded file on the server, unless another playlist still has it.".to_string(),
                    onchange: move |v| {
                        delete_server_file.set(v);
                        server_errors.set(None);
                    },
                }

                ToggleField {
                    label: "Delete download on this device when an episode is removed",
                    checked: delete_client_file(),
                    hint: "Removing an episode from this playlist also removes its downloaded copy from the device that removed it.".to_string(),
                    onchange: move |v| {
                        delete_client_file.set(v);
                        server_errors.set(None);
                    },
                }

                FormSubmit {
                    label: submit_label.to_string(),
                    submitting: submitting(),
                    disabled: submit_disabled,
                    offline: is_offline,
                    offline_hint: (if is_create {
                        "You're offline — reconnect to create a playlist."
                    } else {
                        "You're offline — changes will sync when you reconnect."
                    })
                    .to_string(),
                    errors: catch_all,
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "playlist_form_tests.rs"]
mod tests;
