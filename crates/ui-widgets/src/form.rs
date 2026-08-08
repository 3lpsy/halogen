//! Reactive-form error plumbing — the shared pattern for create/edit forms.
//!
//! [`FormErrors`] normalizes the two error worlds into one `field -> messages`
//! map:
//!   - **local** `validator` errors (built live from the form values; they gate
//!     the submit button and surface under touched fields), and
//!   - **server** [`ApiError`]s (returned on submit).
//!
//! The form renders [`FieldError`] under each input (`for_field`) and
//! [`FormErrorBanner`] below the submit button (`catch_all`) for errors not tied
//! to a form field — database/data/internal/not-found/transport.

use dioxus::prelude::*;

// `FormErrors` lives in `halogen-ui-forms` (pure data, shared with the `use_form` hook in
// ui-state). Re-exported so the `crate::FormErrors` facade — and the
// form pages that use it — stay unchanged.
pub use halogen_ui_forms::FormErrors;

/// Inline validation message(s) under a form field. Renders nothing when empty.
#[component]
pub fn FieldError(messages: Vec<String>) -> Element {
    rsx! {
        if !messages.is_empty() {
            div { class: "mt-1 space-y-0.5",
                for m in messages.iter() {
                    p { class: "text-error text-xs", "{m}" }
                }
            }
        }
    }
}

/// A labelled text field with inline [`FieldError`]s — the shared input shape for
/// the reactive create/edit forms (`pages/playlist_form`, `podcast_config_form`,
/// `user_edit`). Covers `text`/`number`/`password`/`url` inputs and, with
/// `textarea`, a multi-line box.
///
/// `value` is the field's two-way signal; `on_input` fires after every keystroke
/// (forms use it to clear stale server errors as the user fixes a field); the
/// optional `touched` signal is flipped on blur so the caller can gate local
/// "required" errors until the field has been visited. `messages` is the
/// already-merged local+server message list the caller builds via
/// [`FormErrors::field_messages`].
#[component]
pub fn InputField(
    label: String,
    #[props(default = "text".to_string())] input_type: String,
    #[props(default)] placeholder: String,
    #[props(default)] autocomplete: String,
    // e.g. "none" for username fields, where mobile keyboards must not
    // capitalize. Empty (the default) leaves the platform behavior.
    #[props(default)] autocapitalize: String,
    #[props(default)] inputmode: String,
    #[props(default)] hint: String,
    #[props(default)] autofocus: bool,
    #[props(default)] textarea: bool,
    #[props(default = "3".to_string())] rows: String,
    value: Signal<String>,
    #[props(default)] touched: Option<Signal<bool>>,
    #[props(default)] messages: Vec<String>,
    on_input: EventHandler<()>,
) -> Element {
    let mut value = value;
    rsx! {
        div {
            label { class: "label", span { class: "label-text", "{label}" } }
            if textarea {
                textarea {
                    "aria-label": "{label}",
                    class: "textarea textarea-bordered w-full",
                    rows: "{rows}",
                    placeholder: "{placeholder}",
                    autofocus,
                    value: "{value}",
                    oninput: move |e| {
                        value.set(e.value());
                        on_input.call(());
                    },
                    onblur: move |_| {
                        if let Some(mut t) = touched {
                            t.set(true);
                        }
                    },
                }
            } else {
                input {
                    "aria-label": "{label}",
                    class: "input input-bordered w-full",
                    r#type: "{input_type}",
                    autocomplete: "{autocomplete}",
                    autocapitalize: "{autocapitalize}",
                    inputmode: "{inputmode}",
                    placeholder: "{placeholder}",
                    autofocus,
                    value: "{value}",
                    oninput: move |e| {
                        value.set(e.value());
                        on_input.call(());
                    },
                    onblur: move |_| {
                        if let Some(mut t) = touched {
                            t.set(true);
                        }
                    },
                }
            }
            if !hint.is_empty() {
                p { class: "text-xs text-muted mt-1", "{hint}" }
            }
            FieldError { messages }
        }
    }
}

/// Catch-all error banner shown below the submit button for errors not tied to a
/// specific field (database/data/internal/not-found/transport). Renders nothing
/// when empty.
#[component]
pub fn FormErrorBanner(messages: Vec<String>) -> Element {
    rsx! {
        if !messages.is_empty() {
            div { role: "alert", class: "alert alert-error mt-4",
                div { class: "flex flex-col gap-1 text-sm text-left",
                    for m in messages.iter() {
                        span { "{m}" }
                    }
                }
            }
        }
    }
}

/// The trailer every `use_form_state` form ends with: the primary submit button
/// (with an in-flight spinner), an optional offline hint, and the catch-all
/// [`FormErrorBanner`]. `offline_hint` is the per-form message (create vs edit
/// wording differs) shown only while `offline`.
#[component]
pub fn FormSubmit(
    label: String,
    submitting: bool,
    disabled: bool,
    offline: bool,
    offline_hint: String,
    errors: Vec<String>,
) -> Element {
    rsx! {
        button {
            class: "btn btn-primary w-full",
            r#type: "submit",
            disabled,
            if submitting {
                span { class: "loading loading-spinner" }
            }
            "{label}"
        }
        if offline {
            p { class: "text-xs text-muted text-center", "{offline_hint}" }
        }
        FormErrorBanner { messages: errors }
    }
}

/// A labelled checkbox row — `[ ] label` — the shared shape for the boolean
/// toggles across the settings prefs forms. `onchange` receives the new checked
/// state.
#[component]
pub fn CheckboxField(checked: bool, label: String, onchange: EventHandler<bool>) -> Element {
    rsx! {
        div { class: "flex items-center gap-2",
            input {
                r#type: "checkbox",
                class: "checkbox",
                checked,
                onchange: move |e| onchange.call(e.checked()),
            }
            span { class: "text-muted", "{label}" }
        }
    }
}

/// A primary-styled checkbox toggle row — `[x] label` with an optional `hint`
/// paragraph below — the shared shape for the boolean toggles on the create/edit
/// forms (`playlist_form`'s "make default", `podcast_config_form`'s "auto-download",
/// `user_edit`'s "Is Admin"). Distinct from [`CheckboxField`], the muted
/// settings-style checkbox. `onchange` receives the new checked state; `hint` renders
/// only when non-empty.
#[component]
pub fn ToggleField(
    label: String,
    checked: bool,
    #[props(default)] hint: String,
    #[props(default)] disabled: bool,
    onchange: EventHandler<bool>,
) -> Element {
    rsx! {
        div {
            // `whitespace-normal` overrides daisyUI's `.label` nowrap so long labels
            // wrap instead of forcing horizontal scroll; `items-start` for multi-line.
            label { class: "label cursor-pointer justify-start items-start gap-3 whitespace-normal",
                input {
                    r#type: "checkbox",
                    class: "checkbox checkbox-primary",
                    checked,
                    disabled,
                    onchange: move |e| onchange.call(e.checked()),
                }
                span { class: "label-text min-w-0", "{label}" }
            }
            if !hint.is_empty() {
                p { class: "text-xs text-muted mt-1", "{hint}" }
            }
        }
    }
}

/// A labelled `<select>` row for the settings prefs forms: a heading (with an
/// optional `description` paragraph) on the left and the dropdown on the right.
/// `options` are `(value, label)` pairs in display order; `onchange` receives the
/// chosen option's value string (the caller parses it back to its own type).
#[component]
pub fn SelectField(
    label: String,
    #[props(default)] description: String,
    aria_label: String,
    value: String,
    options: Vec<(String, String)>,
    onchange: EventHandler<String>,
    #[props(default = "w-40".to_string())] width_class: String,
    #[props(default)] disabled: bool,
) -> Element {
    let has_desc = !description.is_empty();
    rsx! {
        div {
            class: if has_desc {
                "flex flex-wrap items-start justify-between gap-4"
            } else {
                "flex flex-wrap items-center justify-between gap-4"
            },
            div {
                h3 { class: "text-lg font-medium", "{label}" }
                if has_desc {
                    p { class: "text-sm text-muted", "{description}" }
                }
            }
            select {
                "aria-label": "{aria_label}",
                class: "select select-bordered shrink-0 {width_class}",
                value: "{value}",
                disabled,
                onchange: move |e| onchange.call(e.value()),
                for (val , lbl) in options.iter() {
                    option {
                        key: "{val}",
                        value: "{val}",
                        selected: *val == value,
                        "{lbl}"
                    }
                }
            }
        }
    }
}

/// A settings row: a left-aligned label (with an optional `hint` paragraph) and a
/// right-aligned control (`children`) — the `label … [button]` shape repeated down
/// the Settings page and the other settings screens. `flex-wrap` so the control
/// drops below the label at large UI font scales instead of overflowing the row.
#[component]
pub fn SettingsRow(label: String, #[props(default)] hint: String, children: Element) -> Element {
    rsx! {
        div { class: "flex flex-wrap items-center justify-between gap-2",
            if hint.is_empty() {
                span { class: "text-muted", "{label}" }
            } else {
                // Bounded label block: a long hint wraps INSIDE it instead of
                // claiming the whole row — `justify-between` only right-aligns
                // the control while both share a line.
                div { class: "flex-1 min-w-0 basis-64",
                    span { class: "text-muted", "{label}" }
                    p { class: "text-xs text-muted", "{hint}" }
                }
            }
            // `ml-auto` keeps the control pinned RIGHT even when the row wraps
            // and the control lands on its own flex line.
            div { class: "ml-auto flex items-center gap-2", {children} }
        }
    }
}
