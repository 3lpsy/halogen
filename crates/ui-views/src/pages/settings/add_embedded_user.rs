//! Settings → Accounts → Add embedded user (`/settings/accounts/add-embedded`).
//!
//! Creating an account on the EMBEDDED server never involves a password the
//! user knows: the app generates one, stores it in the server's secrets file
//! for silent login, and registers the user through the admin API. So the form
//! is just a username — plus the admin toggle, visible but locked on (every
//! embedded user is an admin of the on-device server, by policy).
//!
//! There is deliberately NO remote sign-in escape hatch here: embedded and
//! remote are separate worlds, and the supported migration is Settings →
//! Server → Export database, clear this device, sign in remote, Import.

use dioxus::prelude::*;

use super::SettingsSubpage;
use crate::Route;
use crate::components::{FormSubmit, InputField, ToggleField};
use halogen_ui_state::embedded_session::create_embedded_user;
use halogen_ui_state::hooks::{use_accounts, use_toast};

#[component]
pub fn AddEmbeddedUser() -> Element {
    let accounts = use_accounts();
    let toast = use_toast();
    let nav = use_navigator();
    let username = use_signal(String::new);
    let mut error = use_signal(|| None::<String>);
    let loading = use_signal(|| false);

    let onsubmit = move |e: FormEvent| {
        e.prevent_default();
        let name = username();
        let mut err_signal = error;
        let mut loading_signal = loading;
        let mut accounts = accounts;
        loading_signal.set(true);
        err_signal.set(None);

        let _ = spawn(async move {
            match create_embedded_user(accounts, &name).await {
                Ok(reg) => {
                    toast.success(format!("Added {}", name.trim().to_lowercase()));
                    // Same contract as login: navigate first, set the registry
                    // last — the set schedules the keyed remount (the new user
                    // becomes active).
                    let _ = nav.replace(Route::SettingsAccounts {});
                    accounts.set(reg);
                }
                Err(e) => {
                    err_signal.set(Some(e));
                    loading_signal.set(false);
                }
            }
        });
    };

    rsx! {
        SettingsSubpage { title: "Add user",
            div { class: "space-y-4 p-3 bg-base-200 rounded-lg",
                p { class: "text-sm text-muted",
                    "Creates a new account on this device's embedded server and switches to it. No password needed — the app manages sign-in."
                }
                form { class: "space-y-4", onsubmit,
                    InputField {
                        label: "Username",
                        autocomplete: "username",
                        autocapitalize: "none",
                        autofocus: true,
                        value: username,
                        on_input: move |()| error.set(None),
                    }
                    // Visible but locked: every embedded user is an admin of the
                    // on-device server (there is no lesser role to manage here).
                    ToggleField {
                        label: "Administrator",
                        checked: true,
                        disabled: true,
                        hint: "Users on the embedded server are always administrators.",
                        onchange: move |_| {},
                    }
                    FormSubmit {
                        label: "Add user",
                        submitting: loading(),
                        disabled: loading() || username().trim().is_empty(),
                        offline: false,
                        offline_hint: "",
                        errors: error().into_iter().collect::<Vec<_>>(),
                    }
                }
            }
        }
    }
}
