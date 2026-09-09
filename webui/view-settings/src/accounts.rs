//! Settings → Accounts (`/settings/accounts`): switch between signed-in users
//! on this device, add a new one, edit or manage users, or sign one out. Moved
//! verbatim from the old single-page Settings' Accounts section.

use dioxus::prelude::*;

use super::SettingsSubpage;
use halogen_webui_component_icons::User;
use halogen_webui_config::AccountKey;
use halogen_webui_hooks::{use_accounts, use_dispatch, use_is_admin, use_toast};
use halogen_webui_provider_local::embedded_session::{
    sign_out_account_smart, switch_account_smart,
};

/// Switching hot-swaps the active user (the data subtree remounts); each user's
/// cache/queue/history is preserved. Reading the registry subscribes this
/// component, so the list updates after any change.
#[component]
pub fn SettingsAccounts() -> Element {
    let accounts = use_accounts();
    let toast = use_toast();
    let dispatch = use_dispatch();
    let nav = use_navigator();
    let is_admin = use_is_admin();

    // (key, display name, is-active, dom key) per account. The segment doubles
    // as the unique DOM key — ids alone can repeat across server kinds.
    let rows: Vec<(AccountKey, String, bool, String)> = {
        let a = accounts.read();
        let active = a.active_key();
        a.users
            .iter()
            .map(|u| {
                let label = if u.username.is_empty() {
                    format!("User {}", u.id)
                } else {
                    u.username.clone()
                };
                (u.key(), label, Some(u.key()) == active, u.key().segment())
            })
            .collect()
    };

    rsx! {
        SettingsSubpage { title: "Accounts",
            div { class: "space-y-4 p-3 bg-base-200 rounded-lg",
                // Header row: the "Add account" (+ admin "Manage") actions inline
                // on the right.
                div { class: "flex items-center justify-end gap-2 mb-4",
                    // Admin-only: manage all users (list / edit / delete). Left of Add.
                    if is_admin() {
                        button {
                            class: "btn btn-sm btn-primary",
                            onclick: move |_| { nav.push("/admin/users"); },
                            "Manage"
                        }
                    }
                    button {
                        class: "btn btn-sm btn-primary",
                        onclick: move |_| {
                            // Embedded: adding an account CREATES a user on the
                            // in-process server (no password prompt to answer);
                            // remote: the ordinary login flow.
                            if accounts.peek().active_key().is_some_and(|k| k.is_embedded()) {
                                nav.push("/settings/accounts/add-embedded");
                            } else {
                                nav.push("/auth/login");
                            }
                        },
                        "Add account"
                    }
                }
                for (uid, uname, is_active, dom_key) in rows {
                    div {
                        key: "{dom_key}",
                        class: "flex flex-wrap items-center justify-between gap-2",
                        div {
                            class: "flex items-center gap-2",
                            User { class: "w-4 h-4 text-base-content" }
                            span { class: "font-medium", "{uname}" }
                            if uid.is_embedded() {
                                span { class: "badge badge-ghost badge-sm", "This device" }
                            }
                            if is_active {
                                span { class: "badge badge-primary badge-sm", "Active" }
                            }
                        }
                        div {
                            class: "flex items-center gap-2",
                            // Edit (username + password), only the active user, whose namespace + token are mounted,
                            // can self-edit. Left of Sign out. Hidden for embedded accounts: their credentials are
                            // managed internally (silent login), changing them out from under the stored secrets would
                            // only trigger the recovery path.
                            if is_active && !uid.is_embedded() {
                                button {
                                    class: "btn btn-sm btn-primary",
                                    onclick: move |_| { nav.push(format!("/user/{id}/edit", id = uid.id)); },
                                    "Edit"
                                }
                            }
                            if !is_active {
                                button {
                                    class: "btn btn-sm btn-primary",
                                    onclick: move |_| {
                                        let accounts = accounts;
                                        let toast = toast;
                                        spawn(async move { switch_account_smart(accounts, toast, uid).await });
                                    },
                                    "Switch"
                                }
                            }
                            button {
                                class: "btn btn-sm btn-ghost text-error",
                                onclick: move |_| {
                                    let accounts = accounts;
                                    let dispatch = dispatch;
                                    spawn(async move { sign_out_account_smart(accounts, &dispatch, uid).await });
                                },
                                "Sign out"
                            }
                        }
                    }
                }
            }
        }
    }
}
