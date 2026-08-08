//! Edit your own account: username (`PUT /users/{id}`) and password
//! (`POST /auth/password`).
//!
//! Both forms are **online-only** — their submit buttons are disabled while
//! offline and the calls go direct (never through the outbox) so the form can
//! surface server errors inline via the shared [`FormErrors`] surface
//! (`pages/playlist_form` is the app's reference form).
//!
//! The page guards that `id` is the active user. The Settings entry point only
//! shows Edit for the active row, and the server enforces it independently —
//! username via `require_self_or_admin`, password always against the bearer's own
//! account — so this guard is defense-in-depth against a deep link.

use dioxus::prelude::*;
use halogen_wire::{PasswordChangeData, PasswordUpdateData, UserUpdateData, Validate};

use crate::Route;
use crate::components::{FormErrors, FormPage, FormSubmit, InputField, ToggleField};
use halogen_ui_accounts::accounts::AccountsStore;
use halogen_ui_state::hooks::{
    FormState, use_accounts, use_config, use_form_state, use_is_offline, use_toast,
};

/// Username + password forms for the active account.
#[component]
pub fn UserEdit(id: i32) -> Element {
    let accounts = use_accounts();
    let nav = use_navigator();

    // Only the active user may edit here. The server refuses cross-user edits too
    // (403 on a foreign `PUT /users/{id}`; password is always the caller's own), so
    // this is purely a friendlier guard for a deep link.
    if accounts.read().active_user_id != Some(id) {
        return rsx! {
            FormPage {
                div { role: "alert", class: "alert alert-error",
                    span { class: "text-sm", "You can only edit your own account." }
                }
            }
        };
    }

    // Current username for prefill — empty for a migrated account that never
    // backfilled one (this page is how you set it).
    let current_username = accounts
        .read()
        .users
        .iter()
        .find(|u| u.id == id)
        .map(|u| u.username.clone())
        .unwrap_or_default();

    rsx! {
        FormPage {
            h1 { class: "text-2xl font-bold mb-6", "Edit account" }
            div { class: "space-y-8",
                AccountDetailsForm {
                    id,
                    initial: current_username,
                    on_saved: move |_| { nav.replace(Route::Settings {}); },
                }
                PasswordForm {}
            }
        }
    }
}

/// Editable account details — username, plus an optional **Is Admin** toggle when
/// an admin is editing another user. Submits `PUT /users/{id}`. Online-only: the
/// submit button is disabled while offline and the call goes direct (never the
/// outbox) so server errors surface inline.
///
/// `admin` is `Some(current)` only in the admin flow — it renders the checkbox
/// initialised to the user's current flag. It's `None` for self-service edit, where
/// the server forbids changing your own admin flag anyway, so the checkbox is
/// hidden and `is_admin` is never sent. On success the device account registry is
/// updated so a renamed on-device account shows immediately in Settings + the
/// navbar (a no-op when an admin edits a user not signed in on this device), then
/// `on_saved` fires so the caller can navigate (self → Settings, admin → user list).
#[component]
pub fn AccountDetailsForm(
    id: i32,
    initial: String,
    #[props(default)] admin: Option<bool>,
    on_saved: EventHandler<()>,
) -> Element {
    let accounts = use_accounts();
    let config = use_config();
    let toast = use_toast();

    let username = use_signal(|| initial.clone());
    let mut is_admin = use_signal(|| admin.unwrap_or(false));
    let touched = use_signal(|| false);
    let form = use_form_state();
    let FormState {
        submitting,
        mut server_errors,
        mut submitted,
    } = form;

    // The admin toggle only exists in the admin flow; self-edit never sends is_admin.
    let show_admin = admin.is_some();
    let is_offline = use_is_offline()();

    // Live local validation reuses the DTO's own rules (length 3–64); the trimmed
    // value is what we validate AND send.
    let trimmed = username().trim().to_string();
    let payload = UserUpdateData {
        username: Some(trimmed.clone()),
        is_admin: None,
    };
    let local_errors = payload
        .validate()
        .err()
        .map(|e| FormErrors::from_validation(&e))
        .unwrap_or_default();
    // An unchanged form needn't hit the server. Trim both sides so a stored value
    // with stray whitespace doesn't read as "always changed"; the admin flag only
    // counts as a change when the checkbox is shown.
    let username_changed = trimmed != initial.trim();
    let admin_unchanged = admin.map_or(true, |orig| orig == is_admin());
    let unchanged = !username_changed && admin_unchanged;
    // The username only gates the submit (and is only sent) when it's actually being
    // changed: an admin toggling ONLY the is_admin flag on a legacy account that
    // never backfilled a username must not be blocked by that pre-existing invalid
    // value — we send `username: None` for it, leaving the server's copy untouched.
    let username_invalid = username_changed && !local_errors.is_empty();

    let server = server_errors();
    let field_msgs = local_errors.field_messages(
        "username",
        username_changed && (touched() || submitted()),
        server.as_ref(),
    );
    let catch_all: Vec<String> = server
        .as_ref()
        .map(|s| s.catch_all(&["username"]))
        .unwrap_or_default();

    let submit_disabled = submitting() || username_invalid || unchanged || is_offline;

    let on_submit = move |evt: FormEvent| {
        evt.prevent_default();
        submitted.set(true);
        server_errors.set(None);
        if username_invalid || unchanged {
            return;
        }
        // Only send the username when it changed (see `username_invalid` above), so an
        // admin-only flag toggle doesn't re-validate/overwrite a legacy username.
        let new_name = username_changed.then(|| trimmed.clone());
        let new_admin = if show_admin { Some(is_admin()) } else { None };
        form.spawn_submit(
            config,
            move |client| async move {
                client
                    .update_user(
                        id,
                        UserUpdateData {
                            username: new_name,
                            is_admin: new_admin,
                        },
                    )
                    .await
            },
            move |user| async move {
                // Mirror the new name into the device account registry (the source
                // of the Settings + navbar labels). Edit in place so `needs_reauth`
                // and the rest of the slot are preserved. A no-op when the edited
                // user isn't signed in on this device.
                //
                // Slots are keyed `(kind, id, server)` — user ids are per-server,
                // so the edit (which went through the ACTIVE account's server)
                // must only touch the slot on that same server. Matching on the
                // bare id renamed a colliding account from another server (the
                // embedded admin vs a remote admin, both id 1, is the common
                // case).
                let mut accounts = accounts;
                let mut reg = accounts.peek().clone();
                let active = reg.active_key();
                if let Some(slot) = reg.users.iter_mut().find(|u| {
                    u.id == id && active.is_some_and(|a| u.kind == a.kind && u.server == a.server)
                }) {
                    slot.username = user.username.clone();
                }
                AccountsStore::save(&reg).await;
                accounts.set(reg);
                toast.success("Account updated");
                on_saved.call(());
            },
        );
    };

    rsx! {
        form { class: "space-y-4", onsubmit: on_submit,
            h2 { class: "text-lg font-semibold", "Username" }
            InputField {
                label: "Username",
                autocomplete: "username",
                autocapitalize: "none",
                value: username,
                touched: Some(touched),
                messages: field_msgs,
                on_input: move |()| server_errors.set(None),
            }
            if show_admin {
                ToggleField {
                    label: "Is Admin",
                    checked: is_admin(),
                    onchange: move |v| {
                        is_admin.set(v);
                        server_errors.set(None);
                    },
                }
            }
            FormSubmit {
                label: "Save changes",
                submitting: submitting(),
                disabled: submit_disabled,
                offline: is_offline,
                offline_hint: "You're offline — reconnect to save changes.",
                errors: catch_all,
            }
        }
    }
}

/// Change the account's password (`POST /auth/password`). Online-only and never
/// queued. The server re-verifies the current password and that the new/confirm
/// pair match; the id comes from the bearer token, so this only ever changes the
/// caller's own password.
#[component]
fn PasswordForm() -> Element {
    let config = use_config();
    let nav = use_navigator();
    let toast = use_toast();

    let current = use_signal(String::new);
    let new_password = use_signal(String::new);
    let confirm = use_signal(String::new);
    let form = use_form_state();
    let FormState {
        submitting,
        mut server_errors,
        mut submitted,
    } = form;

    let is_offline = use_is_offline()();

    // Local validation mirrors the DTO rules, keyed to the same field names the
    // server uses so inline messages line up (the server stays authoritative —
    // e.g. "Current password is incorrect" arrives keyed `current_password`).
    let local_errors = local_password_errors(&current(), &new_password(), &confirm());
    let invalid = !local_errors.is_empty();

    let server = server_errors();
    let field_msgs = |key: &str| local_errors.field_messages(key, submitted(), server.as_ref());
    let catch_all: Vec<String> = server
        .as_ref()
        .map(|s| s.catch_all(&PASSWORD_FIELDS))
        .unwrap_or_default();

    let submit_disabled = submitting() || invalid || is_offline;

    let on_submit = move |evt: FormEvent| {
        evt.prevent_default();
        submitted.set(true);
        server_errors.set(None);
        if invalid {
            return;
        }
        let data = PasswordChangeData {
            current_password: current(),
            new_password: PasswordUpdateData {
                password: new_password(),
                password_confirm: confirm(),
            },
        };
        form.spawn_submit(
            config,
            move |client| async move { client.change_password(data).await },
            move |()| async move {
                toast.success("Password updated");
                nav.replace(Route::Settings {});
            },
        );
    };

    rsx! {
        form { class: "space-y-4", onsubmit: on_submit,
            h2 { class: "text-lg font-semibold", "Password" }
            InputField {
                label: "Current password",
                input_type: "password",
                autocomplete: "current-password",
                value: current,
                messages: field_msgs("current_password"),
                on_input: move |()| server_errors.set(None),
            }
            InputField {
                label: "New password",
                input_type: "password",
                autocomplete: "new-password",
                value: new_password,
                messages: field_msgs("new_password"),
                on_input: move |()| server_errors.set(None),
            }
            InputField {
                label: "Confirm new password",
                input_type: "password",
                autocomplete: "new-password",
                value: confirm,
                messages: field_msgs("password_confirm"),
                on_input: move |()| server_errors.set(None),
            }
            FormSubmit {
                label: "Update password",
                submitting: submitting(),
                disabled: submit_disabled,
                offline: is_offline,
                offline_hint: "You're offline — reconnect to change your password.",
                errors: catch_all,
            }
        }
    }
}

/// The password form's field keys — mirrors the server validation envelope so the
/// catch-all banner only shows what didn't map to a field.
const PASSWORD_FIELDS: [&str; 3] = ["current_password", "new_password", "password_confirm"];

/// Client-side mirror of the server's password rules (current required; new 8–256
/// chars; confirm matches), keyed to the same fields. The server stays the source
/// of truth — this is for instant feedback and to gate the submit button.
fn local_password_errors(current: &str, new_password: &str, confirm: &str) -> FormErrors {
    let mut errs = FormErrors::default();
    if current.is_empty() {
        errs.push_field("current_password", "Current password is required".into());
    }
    let new_len = new_password.chars().count();
    if !(8..=256).contains(&new_len) {
        errs.push_field(
            "new_password",
            "Password must be between 8 and 256 characters long".into(),
        );
    }
    if confirm != new_password {
        errs.push_field(
            "password_confirm",
            "Password confirmation must match the password".into(),
        );
    }
    errs
}
