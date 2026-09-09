//! Add a local-runtime user with a generated secret stored for silent sign-in. The form asks only for username; local
//! users are always admins. Remote migration uses database export/import rather than adding a remote login here.

use dioxus::prelude::*;

use super::SettingsSubpage;
use crate::components::{FormSubmit, InputField, ToggleField};
use halogen_webui_hooks::{use_accounts, use_toast};
use halogen_webui_provider_local::embedded_session::create_embedded_user;

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
                    let _ = nav.replace("/settings/accounts");
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
