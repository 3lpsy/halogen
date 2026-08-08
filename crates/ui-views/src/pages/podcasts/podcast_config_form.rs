//! Create / edit / remove a podcast's download-poll override config.
//!
//! Follows the playlist form (`pages/playlist_form`) — the app's reference reactive
//! form — and reuses its [`FormErrors`]/[`FieldError`]/[`FormErrorBanner`] surface.
//!
//! One [`PodcastConfigForm`] backs both create and edit. The three override fields
//! are required whole numbers (the server's update is "set only the fields you
//! send", and it can't clear a value to NULL — so the form always sends a full
//! set; reverting to the server's global defaults is the **Remove** action, not an
//! empty field). On create the fields prefill the shared default constants.
//!
//! Submit paths mirror the playlist form: **create** goes direct to the server (it
//! needs the new config id back and links the podcast atomically) and is
//! online-only. **Edit** goes direct when online (so the form can show server
//! errors), or — when offline — applies optimistically and queues an
//! `UpdatePodcastConfig` outbox op. **Remove** behaves like edit (an existing id).

use dioxus::prelude::*;
use halogen_utils::constants::{
    DEFAULT_AUTO_DOWNLOAD_EPISODES_ENABLED, DEFAULT_MAX_CONCURRENT_DOWNLOADS, DEFAULT_MAX_EPISODES,
    DEFAULT_POLL_INTERVAL_SECONDS,
};
use halogen_wire::{PodcastConfigData, PodcastConfigStoreData, PodcastConfigUpdateData, Validate};

use crate::Route;
use crate::components::{ConfirmModal, FormErrors, FormPage, FormSubmit, InputField, ToggleField};
use halogen_ui_logging::warn;
use halogen_ui_state::commands;
use halogen_ui_state::hooks::{
    FormState, use_config, use_connection, use_dispatch, use_form_state, use_podcasts,
};

/// Which flow this form drives.
#[derive(Clone, Copy, PartialEq)]
pub enum FormMode {
    Create,
    Edit { config_id: i32 },
}

/// Create a config for a podcast (`/podcasts/:id/config/create`).
#[component]
pub fn PodcastConfigCreate(id: i32) -> Element {
    rsx! { PodcastConfigForm { podcast_id: id, mode: FormMode::Create } }
}

/// Edit a podcast's config (`/podcasts/:id/config/:config_id/edit`).
#[component]
pub fn PodcastConfigEdit(id: i32, config_id: i32) -> Element {
    rsx! { PodcastConfigForm { podcast_id: id, mode: FormMode::Edit { config_id } } }
}

/// The three config fields, in display order, keyed by their `PodcastConfigData`
/// field name so local + server errors land on the same input.
const FIELD_KEYS: [&str; 3] = [
    "poll_interval_seconds",
    "max_episodes",
    "max_concurrent_downloads",
];

/// `Some(n)` for a non-blank string that parses as `u32`, else `None`.
fn parse_u32(raw: &str) -> Option<u32> {
    let t = raw.trim();
    if t.is_empty() {
        return None;
    }
    t.parse::<u32>().ok()
}

/// Validate the live form values. Range rules come from
/// `PodcastConfigStoreData::validate()` (identical to the update rules);
/// "Enter a whole number" is added here for blank/non-numeric fields. Returns the
/// errors plus the parsed values when everything is valid.
fn validate_fields(
    poll: &str,
    max_eps: &str,
    max_conc: &str,
) -> (FormErrors, Option<(u32, u32, u32)>) {
    let parsed = [parse_u32(poll), parse_u32(max_eps), parse_u32(max_conc)];
    let data = PodcastConfigStoreData {
        poll_interval_seconds: parsed[0],
        max_episodes: parsed[1],
        max_concurrent_downloads: parsed[2],
        // Not range-validated; the checkbox carries it separately on submit.
        auto_download_enabled: None,
    };
    let mut errs = data
        .validate()
        .err()
        .map(|e| FormErrors::from_validation(&e))
        .unwrap_or_default();
    for (i, key) in FIELD_KEYS.iter().enumerate() {
        if parsed[i].is_none() {
            errs.push_field(key, "Enter a whole number".to_string());
        }
    }
    let values = if errs.is_empty() {
        Some((parsed[0].unwrap(), parsed[1].unwrap(), parsed[2].unwrap()))
    } else {
        None
    };
    (errs, values)
}

#[component]
fn PodcastConfigForm(podcast_id: i32, mode: FormMode) -> Element {
    let podcasts = use_podcasts();
    let config = use_config();
    let dispatch = use_dispatch();
    let nav = use_navigator();

    let is_create = matches!(mode, FormMode::Create);
    let connection = use_connection();
    let is_offline = connection.read().is_offline();

    // Field values, prefilled with the shared defaults. On edit these double as the
    // fallback for any override the existing config leaves unset (None).
    let poll = use_signal(|| DEFAULT_POLL_INTERVAL_SECONDS.to_string());
    let max_eps = use_signal(|| DEFAULT_MAX_EPISODES.to_string());
    let max_conc = use_signal(|| DEFAULT_MAX_CONCURRENT_DOWNLOADS.to_string());
    let mut auto_download = use_signal(|| DEFAULT_AUTO_DOWNLOAD_EPISODES_ENABLED);

    // Per-field touched + a submit-attempt flag gate inline errors (never nag a
    // prefilled-valid form before the user touches it).
    let poll_touched = use_signal(|| false);
    let eps_touched = use_signal(|| false);
    let conc_touched = use_signal(|| false);

    // Shared async + server-error scaffolding (submitting / server_errors /
    // submitted). `FormState` is `Copy`, so we keep `form` (for `spawn_submit`)
    // while also destructuring into the same locals the form body uses.
    let form = use_form_state();
    let FormState {
        submitting,
        mut server_errors,
        mut submitted,
    } = form;

    // Edit: prefill once from the cached podcast's config; if it isn't cached yet
    // (deep link), fetch it. Guarded so a later publish can't clobber user edits.
    // `initialized` is set ONLY when real values are loaded — never on a failed
    // fetch — so the form can't submit defaults over the server's config.
    let mut initialized = use_signal(|| false);
    let mut load_failed = use_signal(|| false);
    // In-flight guard: this effect subscribes to reactive `podcasts` (which ticks
    // during playback/downloads), so without it a cold deep-link edit would spawn a
    // duplicate `get_podcast_config` on every tick until `initialized` flips.
    let mut fetching = use_signal(|| false);
    use_effect(move || {
        if initialized() || *fetching.peek() {
            return;
        }
        // Subscribe to connectivity so a reconnect re-runs this and retries the fetch
        // — otherwise a failed offline deep-link prefill stays stuck on the "check
        // your connection" alert until some unrelated PodcastState publish.
        let _ = connection.read().is_offline();
        let FormMode::Edit { config_id } = mode else {
            initialized.set(true);
            return;
        };
        let fill = move |cfg: PodcastConfigData| {
            let mut poll = poll;
            let mut max_eps = max_eps;
            let mut max_conc = max_conc;
            let mut auto_download = auto_download;
            poll.set(
                cfg.poll_interval_seconds
                    .unwrap_or(DEFAULT_POLL_INTERVAL_SECONDS)
                    .to_string(),
            );
            max_eps.set(cfg.max_episodes.unwrap_or(DEFAULT_MAX_EPISODES).to_string());
            max_conc.set(
                cfg.max_concurrent_downloads
                    .unwrap_or(DEFAULT_MAX_CONCURRENT_DOWNLOADS)
                    .to_string(),
            );
            auto_download.set(
                cfg.auto_download_enabled
                    .unwrap_or(DEFAULT_AUTO_DOWNLOAD_EPISODES_ENABLED),
            );
        };
        if let Some(cfg) = podcasts
            .read()
            .podcast(podcast_id)
            .and_then(|p| p.podcast_config.clone())
        {
            fill(cfg);
            initialized.set(true);
            return;
        }
        // Not cached — fetch the config directly. On failure we mark `load_failed`
        // (NOT `initialized`): a deep-link edit while offline must not leave the
        // form showing defaults that a submit would write over the real config.
        let cfg_conn = config.peek().clone();
        fetching.set(true);
        spawn(async move {
            let Some(client) = cfg_conn.api_client() else {
                load_failed.set(true);
                fetching.set(false);
                return;
            };
            // A new attempt: clear the previous failure so a retry shows "loading".
            load_failed.set(false);
            match client.get_podcast_config(config_id).await {
                Ok(cfg) => {
                    fill(cfg);
                    initialized.set(true);
                }
                Err(e) => {
                    warn!(config_id, error = %e, "Edit prefill: config fetch failed");
                    load_failed.set(true);
                }
            }
            fetching.set(false);
        });
    });

    // Live local validation → gates submit + inline field errors.
    let (local_errors, _) = validate_fields(&poll(), &max_eps(), &max_conc());
    let invalid = !local_errors.is_empty();

    let server = server_errors();
    let field_msgs = |key: &str, touched: bool| {
        local_errors.field_messages(key, touched || submitted(), server.as_ref())
    };
    let catch_all: Vec<String> = server
        .as_ref()
        .map(|s| s.catch_all(&FIELD_KEYS))
        .unwrap_or_default();

    // Edit can't submit until the real config has loaded — otherwise it would
    // overwrite the server's values with the prefilled defaults. Create has nothing
    // to prefill, so it's ready immediately.
    let edit_ready = is_create || initialized();
    // Create needs a real server id, so it's online-only; edit works offline.
    let submit_disabled = submitting() || invalid || (is_create && is_offline) || !edit_ready;
    let submit_label = if is_create {
        "Create polling config"
    } else {
        "Save changes"
    };
    let title = if is_create {
        "New polling config"
    } else {
        "Edit polling config"
    };

    let on_submit = move |evt: FormEvent| {
        evt.prevent_default();
        submitted.set(true);
        server_errors.set(None);
        let (_, values) = validate_fields(&poll(), &max_eps(), &max_conc());
        let Some((p, me, mc)) = values else {
            return;
        };

        // Offline EDIT: optimistic apply + outbox, then leave. Offline CREATE can't
        // happen (button disabled). Online: direct API below.
        if is_offline {
            if let FormMode::Edit { config_id } = mode {
                commands::update_podcast_config(
                    &dispatch,
                    podcast_id,
                    config_id,
                    PodcastConfigUpdateData {
                        poll_interval_seconds: Some(p),
                        max_episodes: Some(me),
                        max_concurrent_downloads: Some(mc),
                        auto_download_enabled: Some(auto_download()),
                    },
                );
                nav.replace(Route::PodcastDetail { id: podcast_id });
            }
            return;
        }

        let auto = auto_download();
        form.spawn_submit(
            config,
            move |client| async move {
                match mode {
                    FormMode::Create => {
                        client
                            .create_podcast_config_for(
                                podcast_id,
                                PodcastConfigStoreData {
                                    poll_interval_seconds: Some(p),
                                    max_episodes: Some(me),
                                    max_concurrent_downloads: Some(mc),
                                    auto_download_enabled: Some(auto),
                                },
                            )
                            .await
                    }
                    FormMode::Edit { config_id } => {
                        client
                            .update_podcast_config(
                                config_id,
                                PodcastConfigUpdateData {
                                    poll_interval_seconds: Some(p),
                                    max_episodes: Some(me),
                                    max_concurrent_downloads: Some(mc),
                                    auto_download_enabled: Some(auto),
                                },
                            )
                            .await
                    }
                }
            },
            move |new_cfg| async move {
                // Optimistically merge the config into the cached podcast so the
                // detail page reflects it instantly (no re-fetch needed).
                if let Some(mut pod) = podcasts.peek().podcast(podcast_id).cloned() {
                    pod.podcast_config_id = Some(new_cfg.id);
                    pod.podcast_config = Some(new_cfg);
                    commands::cache_podcasts(&dispatch, vec![pod]);
                }
                nav.replace(Route::PodcastDetail { id: podcast_id });
            },
        );
    };

    rsx! {
        FormPage {
            h1 { class: "text-2xl font-bold mb-1", "{title}" }
            p { class: "text-sm text-muted mb-4",
                "Override how often this podcast is polled and how its episodes download. Leave the defaults to match the server."
            }

            if !edit_ready {
                // Edit prefill hasn't loaded yet. Show loading vs a clear
                // failure instead of an editable form full of defaults that a
                // submit could write over the real server config.
                if load_failed() {
                    div { role: "alert", class: "alert alert-error",
                        span { class: "text-sm",
                            if is_offline {
                                "You're offline — reconnect to edit this config."
                            } else {
                                "Couldn't load this config. Check your connection and try again."
                            }
                        }
                    }
                } else {
                    div { class: "flex items-center gap-2 text-muted",
                        span { class: "loading loading-spinner loading-sm" }
                        "Loading config…"
                    }
                }
            } else {
                form { class: "space-y-4", onsubmit: on_submit,
                    InputField {
                        label: "Poll interval (seconds)",
                        input_type: "number",
                        inputmode: "numeric",
                        hint: "How often to check this feed for new episodes (0–86400). The server still only wakes on its own cadence.",
                        autofocus: true,
                        value: poll,
                        touched: Some(poll_touched),
                        messages: field_msgs("poll_interval_seconds", poll_touched()),
                        on_input: move |()| server_errors.set(None),
                    }
                    InputField {
                        label: "Max episodes",
                        input_type: "number",
                        inputmode: "numeric",
                        hint: "How many downloaded episodes to keep on the server (1–10000). When exceeded, the oldest download is removed.",
                        value: max_eps,
                        touched: Some(eps_touched),
                        messages: field_msgs("max_episodes", eps_touched()),
                        on_input: move |()| server_errors.set(None),
                    }
                    InputField {
                        label: "Max concurrent downloads",
                        input_type: "number",
                        inputmode: "numeric",
                        hint: "Parallel auto-downloads for this podcast (1–100).",
                        value: max_conc,
                        touched: Some(conc_touched),
                        messages: field_msgs("max_concurrent_downloads", conc_touched()),
                        on_input: move |()| server_errors.set(None),
                    }

                    // Auto-download new episodes server-side.
                    ToggleField {
                        label: "Auto-download new episodes",
                        checked: auto_download(),
                        hint: "When on, the server downloads new episodes as they're polled and keeps the newest “max episodes”.",
                        onchange: move |v| {
                            auto_download.set(v);
                            server_errors.set(None);
                        },
                    }

                    FormSubmit {
                        label: submit_label.to_string(),
                        submitting: submitting(),
                        disabled: submit_disabled,
                        offline: is_offline,
                        offline_hint: (if is_create {
                            "You're offline — reconnect to create a config."
                        } else {
                            "You're offline — changes will sync when you reconnect."
                        })
                        .to_string(),
                        errors: catch_all,
                    }
                }

                // Remove (edit only) — reverts the podcast to the server's global
                // defaults. Confirmed via a modal.
                if let FormMode::Edit { config_id } = mode {
                    RemoveConfigSection { podcast_id, config_id }
                }
            }
        }
    }
}

/// The danger-zone "Remove config" control + its confirmation modal.
#[component]
fn RemoveConfigSection(podcast_id: i32, config_id: i32) -> Element {
    let podcasts = use_podcasts();
    let config = use_config();
    let dispatch = use_dispatch();
    let nav = use_navigator();

    let is_offline = use_connection().read().is_offline();
    let mut confirm = use_signal(|| false);
    let mut removing = use_signal(|| false);
    let mut error = use_signal(|| Option::<String>::None);

    let do_remove = move |_: ()| {
        error.set(None);
        // Offline: optimistic unlink + outbox, then leave.
        if is_offline {
            commands::remove_podcast_config(&dispatch, podcast_id, config_id);
            confirm.set(false);
            nav.replace(Route::PodcastDetail { id: podcast_id });
            return;
        }
        removing.set(true);
        let cfg = config.peek().clone();
        spawn(async move {
            let Some(client) = cfg.api_client() else {
                removing.set(false);
                error.set(Some("No server configured.".into()));
                return;
            };
            let result = client.remove_podcast_config_for(podcast_id).await;
            removing.set(false);
            match result {
                Ok(()) => {
                    // Optimistically drop the config from the cached podcast.
                    if let Some(mut pod) = podcasts.peek().podcast(podcast_id).cloned() {
                        pod.podcast_config_id = None;
                        pod.podcast_config = None;
                        commands::cache_podcasts(&dispatch, vec![pod]);
                    }
                    confirm.set(false);
                    nav.replace(Route::PodcastDetail { id: podcast_id });
                }
                Err(e) => error.set(Some(
                    FormErrors::from_api_error(&e)
                        .catch_all(&[])
                        .first()
                        .cloned()
                        .unwrap_or_else(|| "Couldn't remove the config.".into()),
                )),
            }
        });
    };

    rsx! {
        div { class: "mt-8 pt-4 border-t border-base-200",
            button {
                class: "btn btn-error btn-outline btn-sm",
                r#type: "button",
                onclick: move |_| confirm.set(true),
                "Remove config"
            }
            p { class: "text-xs text-muted mt-1",
                "Deletes this override and reverts the podcast to the server's global defaults."
            }
            if let Some(msg) = error() {
                div { role: "alert", class: "alert alert-error mt-3",
                    span { class: "text-sm", "{msg}" }
                }
            }
        }

        if confirm() {
            ConfirmModal {
                title: "Remove polling config?",
                body: "This podcast will go back to the server's global download/poll defaults.",
                confirm_label: "Remove",
                danger: true,
                busy: removing(),
                title_id: "confirm-remove-config-title",
                on_cancel: move |_| confirm.set(false),
                on_confirm: do_remove,
            }
        }
    }
}
