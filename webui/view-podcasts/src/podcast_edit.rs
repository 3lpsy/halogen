//! Edit a podcast's own fields (`/podcasts/:id/edit`), title, description, and feed URL. Modeled on
//! `pages/playlists/playlist_form`. Online-only: unlike playlist edits there is no podcast-update outbox op, and a
//! feed-URL change should surface server validation immediately (the next poll fetches the new URL). On success the
//! updated row is cached into the pool and the form navigates back to the podcast detail.

use dioxus::prelude::*;
use halogen_wire::{PodcastUpdateData, Validate};

use crate::components::{FormErrors, FormPage, FormSubmit, InputField};
use halogen_webui_commands::actions as commands;
use halogen_webui_hooks::{
    FormState, use_config, use_connection, use_deep_link_resource, use_dispatch, use_form_state,
    use_podcasts,
};

fn update_data(title: &str, description: &str, feed_url: &str) -> PodcastUpdateData {
    PodcastUpdateData {
        title: Some(title.to_string()),
        description: Some(description.trim().to_string()),
        feed_url: Some(feed_url.trim().to_string()),
    }
}

/// Validate the live form values against the typed update payload. The empty
/// result means valid.
fn validate_fields(title: &str, description: &str, feed_url: &str) -> FormErrors {
    update_data(title, description, feed_url)
        .validate()
        .err()
        .map(|e| FormErrors::from_validation(&e))
        .unwrap_or_default()
}

#[component]
pub fn PodcastEdit(id: i32) -> Element {
    let podcasts = use_podcasts();
    let config = use_config();
    let dispatch = use_dispatch();
    let nav = use_navigator();

    let is_offline = use_connection().read().is_offline();

    // Field values.
    let mut title = use_signal(String::new);
    let mut description = use_signal(String::new);
    let mut feed_url = use_signal(String::new);

    // Local-error gating: a field's error shows once it's been touched (blurred)
    // or after a submit attempt.
    let title_touched = use_signal(|| false);
    let feed_url_touched = use_signal(|| false);

    // Shared async + server-error scaffolding (submitting / server_errors /
    // submitted) — same trio as the playlist form.
    let form = use_form_state();
    let FormState {
        submitting,
        mut server_errors,
        mut submitted,
    } = form;

    // Deep-link: podcasts aren't bulk-held, so a hard refresh of
    // `/podcasts/:id/edit` lands with a cold pool — fetch + cache so the prefill
    // below can populate the form (mirrors `PodcastDetail`).
    let _ = use_deep_link_resource(
        config,
        move || podcasts.read().podcast(id).is_some(),
        move |client| async move {
            let p = client.get_podcast(id).await.map_err(|e| e.to_string())?;
            commands::cache_podcasts(&dispatch, vec![p]);
            Ok(())
        },
    );

    // Prefill once the podcast lands in the pool. Guarded so a later publish (or
    // a keystroke-triggered re-render) never clobbers the user's edits.
    let mut initialized = use_signal(|| false);
    use_effect(move || {
        if initialized() {
            return;
        }
        if let Some(p) = podcasts.read().podcast(id).cloned() {
            title.set(p.title);
            description.set(p.description);
            feed_url.set(p.feed_url);
            initialized.set(true);
        }
    });

    // Live local validation → gates the submit button + drives inline field errors.
    let local_errors = validate_fields(&title(), &description(), &feed_url());
    let invalid = !local_errors.is_empty();

    // Inline messages: local (only once touched / submitted) plus any server field
    // error. Server errors are cleared on edit, so they never linger past a fix.
    let server = server_errors();
    let show_local_title = title_touched() || submitted();
    let show_local_feed = feed_url_touched() || submitted();
    let title_messages = local_errors.field_messages("title", show_local_title, server.as_ref());
    let description_messages = local_errors.field_messages("description", false, server.as_ref());
    let feed_url_messages =
        local_errors.field_messages("feed_url", show_local_feed, server.as_ref());
    // Everything that isn't a form field → the catch-all banner below the button.
    let catch_all: Vec<String> = server
        .as_ref()
        .map(|s| s.catch_all(&["title", "description", "feed_url"]))
        .unwrap_or_default();

    // Online-only (no podcast-update outbox op), and never before the existing
    // values have prefilled — a save on a cold-cache deep link would write blank
    // form defaults over the real podcast.
    let submit_disabled = submitting() || invalid || is_offline || !initialized();

    let on_submit = move |evt: FormEvent| {
        evt.prevent_default();
        submitted.set(true);
        server_errors.set(None);
        // Never submit before the values loaded (see `submit_disabled`).
        if !initialized() {
            return;
        }
        if !validate_fields(&title(), &description(), &feed_url()).is_empty() {
            return;
        }

        let data = update_data(&title(), &description(), &feed_url());

        form.spawn_submit(
            config,
            move |client| async move { client.update_podcast(id, data).await },
            move |p| async move {
                // Optimistically cache the updated row (instant detail render),
                // then reconcile via a pull.
                commands::cache_podcasts(&dispatch, vec![p]);
                commands::refresh(&dispatch);
                nav.replace(format!("/podcasts/{id}", id = id));
            },
        );
    };

    rsx! {
        FormPage {
            h1 { class: "text-2xl font-bold mb-4", "Edit podcast" }

            form { class: "space-y-4", onsubmit: on_submit,
                // Title (required).
                InputField {
                    label: "Title",
                    placeholder: "Podcast title",
                    value: title,
                    touched: Some(title_touched),
                    messages: title_messages,
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

                // Feed URL (required) — the point of this form: repoint a
                // subscription whose feed moved.
                InputField {
                    label: "Feed URL",
                    placeholder: "https://example.com/feed.xml",
                    inputmode: "url",
                    value: feed_url,
                    touched: Some(feed_url_touched),
                    messages: feed_url_messages,
                    hint: "The next poll fetches from this URL.",
                    on_input: move |()| server_errors.set(None),
                }

                FormSubmit {
                    label: "Save changes",
                    submitting: submitting(),
                    disabled: submit_disabled,
                    offline: is_offline,
                    offline_hint: "You're offline — reconnect to edit a podcast.",
                    errors: catch_all,
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "podcast_edit_tests.rs"]
mod tests;
