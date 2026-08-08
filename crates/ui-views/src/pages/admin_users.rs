//! Admin-only user management: list every user (`GET /users`), create one
//! (`POST /admin/users` + [`AdminUserCreate`]), edit one (`GET /users/{id}` +
//! the shared [`AccountDetailsForm`]), or delete one (`DELETE /users/{id}`).
//!
//! These pages are **online-only** — no outbox, no local caching, no lazy paging.
//! Each render fetches live via `use_resource`; every mutating control is disabled
//! while offline (and the server is the real authority — the admin gate, the
//! self-delete refusal, and the is_admin guard all live there). The Settings →
//! Accounts "Manage" button (admin-only) is the entry point.

use dioxus::prelude::*;
use halogen_wire::{DefaultListParams, NoInclude, Pagination, UserData, UserStoreData, Validate};

use crate::Route;
use crate::components::{
    BackButton, ConfirmModal, FormErrors, FormPage, FormSubmit, InputField, ToggleField,
    resource_list_view, resource_view,
};
use crate::pages::user_edit::AccountDetailsForm;
use halogen_ui_state::hooks::{
    FormState, use_accounts, use_config, use_form_state, use_is_admin, use_is_offline, use_toast,
};

/// One page is enough for the whole list — `GET /users` is paginated, but admin
/// user counts are tiny, so we request the max page size and skip lazy loading.
const MAX_USERS: i32 = 65536;

/// Admin user list. Online-only; rows carry Edit + Delete actions. You can't delete
/// your own account (the button is disabled — another admin must do it), and every
/// action is disabled while offline.
#[component]
pub fn AdminUsers() -> Element {
    let cfg = use_config();
    let accounts = use_accounts();
    let toast = use_toast();
    let nav = use_navigator();

    let is_offline = use_is_offline()();
    let own_id = accounts.read().active_user_id;

    // Live fetch — re-runs when the session changes and on `restart()` after a
    // delete. `api_client_or_err` returns Err when unconfigured OR manually offline.
    let mut users = use_resource(move || {
        let client = cfg.read().api_client_or_err();
        // `NoInclude` has no `Default`, so build the params by hand: one max-size
        // page (no lazy loading), default order, no includes/filters.
        let params = DefaultListParams::<NoInclude> {
            pagination: Some(Pagination {
                page: 0,
                size: MAX_USERS,
            }),
            order: None,
            includes: None,
            filter: None,
        };
        async move {
            let client = client?;
            client
                .list_users(params)
                .await
                .map(|page| page.data)
                .map_err(|e| e.to_string())
        }
    });

    // The user pending deletion (id, display name) — drives the confirm modal.
    let mut pending_delete = use_signal(|| None::<(i32, String)>);
    let mut deleting = use_signal(|| false);

    rsx! {
        div { class: "p-2 space-y-6",
            BackButton {}
            div { class: "flex items-center justify-between",
                h1 { class: "text-3xl font-bold", "Users" }
                div { class: "flex items-center gap-2",
                    button {
                        class: "btn btn-ghost btn-sm",
                        disabled: is_offline,
                        onclick: move |_| users.restart(),
                        "Refresh"
                    }
                    button {
                        class: "btn btn-primary btn-sm",
                        disabled: is_offline,
                        onclick: move |_| { nav.push(Route::AdminUserCreate {}); },
                        "Create user"
                    }
                }
            }

            if is_offline {
                div { role: "alert", class: "alert alert-warning",
                    span { "You're offline — reconnect to manage users." }
                }
            }

            {
                resource_list_view(
                    &*users.read_unchecked(),
                    "users",
                    "No users.",
                    "flex flex-col gap-2",
                    |user: UserData| rsx! {
                        UserRow {
                            key: "{user.id}",
                            is_self: own_id == Some(user.id),
                            is_offline,
                            on_edit: move |id| { nav.push(Route::AdminUserEdit { id }); },
                            on_delete: move |(id, name)| pending_delete.set(Some((id, name))),
                            user,
                        }
                    },
                )
            }
        }

        if let Some((id, name)) = pending_delete() {
            ConfirmModal {
                title: "Delete user?",
                body: format!("This permanently deletes the account \"{name}\". This can't be undone."),
                confirm_label: "Delete",
                danger: true,
                busy: deleting(),
                on_cancel: move |_| pending_delete.set(None),
                on_confirm: move |_| {
                    let client = cfg.read().api_client_or_err();
                    deleting.set(true);
                    spawn(async move {
                        let result = match client {
                            Ok(c) => c.delete_user(id).await.map_err(|e| e.to_string()),
                            Err(e) => Err(e),
                        };
                        deleting.set(false);
                        pending_delete.set(None);
                        match result {
                            Ok(()) => {
                                toast.success("User deleted");
                                users.restart();
                            }
                            Err(e) => toast.error(format!("Delete failed: {e}")),
                        }
                    });
                },
            }
        }
    }
}

/// A single user row: name + admin badge, with Edit and Delete actions. Delete is
/// disabled for your own account (another admin must remove it) and both actions
/// are disabled while offline.
#[component]
fn UserRow(
    user: UserData,
    is_self: bool,
    is_offline: bool,
    on_edit: EventHandler<i32>,
    on_delete: EventHandler<(i32, String)>,
) -> Element {
    let label = if user.username.is_empty() {
        format!("User {}", user.id)
    } else {
        user.username.clone()
    };
    let id = user.id;
    let name = label.clone();

    rsx! {
        div { class: "flex flex-wrap items-center justify-between gap-2 p-3 bg-base-200 rounded-lg",
            div { class: "flex items-center gap-2",
                span { class: "font-medium", "{label}" }
                if user.is_admin {
                    span { class: "badge badge-primary badge-sm", "Admin" }
                }
                if is_self {
                    span { class: "badge badge-ghost badge-sm", "You" }
                }
            }
            div { class: "flex items-center gap-2",
                button {
                    class: "btn btn-sm btn-primary",
                    disabled: is_offline,
                    onclick: move |_| on_edit.call(id),
                    "Edit"
                }
                button {
                    class: "btn btn-sm btn-ghost text-error",
                    // Can't delete your own account — another admin must do it.
                    disabled: is_offline || is_self,
                    onclick: move |_| on_delete.call((id, name.clone())),
                    "Delete"
                }
            }
        }
    }
}

/// Create a new server user with a password (`POST /admin/users`), reached from
/// the [`AdminUsers`] header. Online-only like the rest of this module: the submit
/// button is disabled while offline and the call goes direct (never the outbox) so
/// server errors surface inline via the shared [`FormErrors`] surface.
///
/// Local validation reuses [`UserStoreData`]'s own rules (username 3–64 chars,
/// password 8–256, confirmation must match), gating the submit button and showing
/// inline field errors once a field is touched / after a submit attempt. The server
/// enforces admin-only access independently; this `is_admin` check is a friendly
/// guard for a deep link. On success: a toast, then back to the user list.
#[component]
pub fn AdminUserCreate() -> Element {
    let config = use_config();
    let toast = use_toast();
    let nav = use_navigator();
    let viewer_is_admin = use_is_admin();

    // Field values. `make_admin` backs the Is Admin toggle — default OFF, so a
    // slip of the finger can't mint an admin.
    let username = use_signal(String::new);
    let password = use_signal(String::new);
    let confirm = use_signal(String::new);
    let mut make_admin = use_signal(|| false);
    // The username error shows once the field blurs (or after a submit attempt);
    // the password fields wait for a submit, like `PasswordForm` in `user_edit`.
    let username_touched = use_signal(|| false);
    let form = use_form_state();
    let FormState {
        submitting,
        mut server_errors,
        mut submitted,
    } = form;

    let is_offline = use_is_offline()();

    // Keep every hook ABOVE this guard: `use_is_admin` resolves asynchronously and
    // can flip false→true while mounted (see the hook-ordering note on
    // [`AdminUserEdit`]).
    if !viewer_is_admin() {
        return rsx! {
            FormPage {
                div { role: "alert", class: "alert alert-error",
                    span { class: "text-sm", "Admins only." }
                }
            }
        };
    }

    // Live local validation reuses the DTO's own rules; the trimmed username is
    // what we validate AND send (the passwords go verbatim — leading/trailing
    // whitespace is legal in a password).
    let payload = UserStoreData {
        username: username().trim().to_string(),
        password: password(),
        password_confirm: confirm(),
        is_admin: Some(make_admin()),
    };
    let local_errors = payload
        .validate()
        .err()
        .map(|e| FormErrors::from_validation(&e))
        .unwrap_or_default();
    let invalid = !local_errors.is_empty();

    // Inline messages: local (only once touched / submitted) plus any server field
    // error. Server errors are cleared on edit, so they never linger past a fix.
    let server = server_errors();
    let username_msgs = local_errors.field_messages(
        "username",
        username_touched() || submitted(),
        server.as_ref(),
    );
    let password_msgs = local_errors.field_messages("password", submitted(), server.as_ref());
    let confirm_msgs =
        local_errors.field_messages("password_confirm", submitted(), server.as_ref());
    // Everything that isn't a form field → the catch-all banner below the button.
    let catch_all: Vec<String> = server
        .as_ref()
        .map(|s| s.catch_all(&["username", "password", "password_confirm"]))
        .unwrap_or_default();

    let submit_disabled = submitting() || invalid || is_offline;

    let on_submit = move |evt: FormEvent| {
        evt.prevent_default();
        submitted.set(true);
        server_errors.set(None);
        // Rebuild from the live signals (the render-scope `payload` isn't
        // captured) and re-check — belt-and-braces against a stale-render submit.
        let data = UserStoreData {
            username: username().trim().to_string(),
            password: password(),
            password_confirm: confirm(),
            is_admin: Some(make_admin()),
        };
        if data.validate().is_err() {
            return;
        }
        form.spawn_submit(
            config,
            move |client| async move { client.create_user(data).await },
            move |_user| async move {
                toast.success("User created");
                nav.replace(Route::AdminUsers {});
            },
        );
    };

    rsx! {
        FormPage {
            h1 { class: "text-2xl font-bold mb-6", "Create user" }
            form { class: "space-y-4", onsubmit: on_submit,
                InputField {
                    label: "Username",
                    autocomplete: "username",
                    autocapitalize: "none",
                    autofocus: true,
                    value: username,
                    touched: Some(username_touched),
                    messages: username_msgs,
                    on_input: move |()| server_errors.set(None),
                }
                InputField {
                    label: "Password",
                    input_type: "password",
                    autocomplete: "new-password",
                    value: password,
                    messages: password_msgs,
                    on_input: move |()| server_errors.set(None),
                }
                InputField {
                    label: "Confirm password",
                    input_type: "password",
                    autocomplete: "new-password",
                    value: confirm,
                    messages: confirm_msgs,
                    on_input: move |()| server_errors.set(None),
                }
                ToggleField {
                    label: "Is Admin",
                    checked: make_admin(),
                    onchange: move |v| {
                        make_admin.set(v);
                        server_errors.set(None);
                    },
                }
                FormSubmit {
                    label: "Create user",
                    submitting: submitting(),
                    disabled: submit_disabled,
                    offline: is_offline,
                    offline_hint: "You're offline — reconnect to create a user.",
                    errors: catch_all,
                }
            }
        }
    }
}

/// Admin edit page for another user — reuses [`AccountDetailsForm`] with the
/// Is Admin toggle enabled. Online-only: the target is fetched live via
/// `GET /users/{id}`. There's no password field here (admins can't change another
/// user's password). The server enforces admin-only access independently; this
/// `is_admin` check is a friendly guard for a deep link.
#[component]
pub fn AdminUserEdit(id: i32) -> Element {
    let cfg = use_config();
    let is_admin = use_is_admin();
    let nav = use_navigator();

    // `use_resource` is a hook — keep it ABOVE the admin guard so the hook count is
    // stable. `use_is_admin` resolves asynchronously and can flip false→true while
    // mounted; a hook after the early-returning guard would corrupt hook indices on
    // that flip. (A non-admin deep link triggers one harmless `get_user` the server
    // rejects; the guard below returns before the result is ever rendered.)
    let user = use_resource(move || {
        let client = cfg.read().api_client_or_err();
        async move {
            let client = client?;
            client.get_user(id).await.map_err(|e| e.to_string())
        }
    });

    if !is_admin() {
        return rsx! {
            FormPage {
                div { role: "alert", class: "alert alert-error",
                    span { class: "text-sm", "Admins only." }
                }
            }
        };
    }

    rsx! {
        FormPage {
            h1 { class: "text-2xl font-bold mb-6", "Edit user" }
            {
                resource_view(&*user.read_unchecked(), "user", |u: &UserData| {
                    rsx! {
                        AccountDetailsForm {
                            id: u.id,
                            initial: u.username.clone(),
                            admin: Some(u.is_admin),
                            on_saved: move |_| { nav.replace(Route::AdminUsers {}); },
                        }
                    }
                })
            }
        }
    }
}
