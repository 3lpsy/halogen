//! `/auth/embedded-setup` — the full-page confirmation for **Embedded Server**
//! mode: describes the trade-offs of running the built-in server on this
//! device, then provisions it (boot → silent local sign-in) on confirm.
//!
//! Reached from the "Use Embedded Server" link on the Login page (rendered
//! only when the build carries the embedded server), and by `RootGuard` when
//! an embedded account needs to reconnect (its credentials live on disk — a
//! password prompt is never the answer for embedded accounts).

use dioxus::prelude::*;

use super::card::AuthCard;
use crate::Route;
use crate::root_guard::PendingAuthRedirect;
use halogen_ui_state::hooks::use_accounts;
use halogen_ui_state::{embedded, embedded_session};

#[component]
pub fn EmbeddedServerSetup() -> Element {
    let pending_redirect = use_context::<PendingAuthRedirect>();
    let accounts = use_accounts();
    let error = use_signal(|| None::<String>);
    let loading = use_signal(|| false);
    let nav = use_navigator();

    // Same library, second visit (reconnect / re-auth) vs first-time creation —
    // the copy must make clear that reconnecting deletes nothing.
    let reconnecting = embedded::library_exists();

    let onsubmit = move |e: FormEvent| {
        e.prevent_default();
        let mut err_signal = error;
        let mut loading_signal = loading;
        let mut accounts = accounts;
        loading_signal.set(true);

        let _ = spawn(async move {
            let mut bail = move |msg: String| {
                err_signal.set(Some(msg));
                loading_signal.set(false);
            };

            // Boot + silent sign-in with the on-disk credentials; a Retry is
            // just submitting again. Full detail lands in the device log.
            match embedded_session::prepare_embedded_activation(accounts).await {
                Ok(reg) => {
                    // Same contract as Login: navigate first, set the registry
                    // last — the set schedules the keyed remount that tears
                    // this page down.
                    // Land back on the deep link the auth redirect displaced
                    // (falls back to Home).
                    let _ = nav.replace(pending_redirect.take_or_home());
                    accounts.set(reg);
                }
                Err(e) => bail(e),
            }
        });
    };

    if !embedded::available() {
        // Unreachable through the gated link; covers a deep link on web or a
        // build without the feature.
        return rsx! {
            AuthCard {
                subtitle: "Embedded server",
                max_width: "max-w-md",
                error,
                loading,
                submit_label: "Back to sign in",
                pending_label: "…",
                onsubmit: move |e: FormEvent| {
                    e.prevent_default();
                    let _ = nav.replace(Route::Login {});
                },
                p { class: "text-sm text-muted",
                    "The embedded server isn't available in this build. Connect to a server instead."
                }
            }
        };
    }

    rsx! {
        AuthCard {
            subtitle: "Run Halogen on this device only",
            max_width: "max-w-md",
            error,
            loading,
            submit_label: if reconnecting { "Reconnect to Embedded Server" } else { "Use Embedded Server" },
            pending_label: "Starting embedded server...",
            onsubmit,
            on_back: move |_| {
                let _ = nav.replace(Route::Login {});
            },
            if reconnecting {
                div { role: "alert", class: "alert alert-info text-sm",
                    span {
                        "An embedded library already exists on this device — you'll reconnect to it. Nothing is deleted."
                    }
                }
            } else {
                p { class: "text-sm",
                    "The app will run its own built-in server. Good for trying Halogen out or for a single-device setup. Compared to a dedicated server:"
                }
            }
            ul { class: "list-disc list-outside pl-5 space-y-2 text-sm text-muted",
                li {
                    strong { class: "text-base-content", "This device only. " }
                    "Subscriptions, playback history and downloads live in this app on this device — no sync to other devices or the web app."
                }
                li {
                    strong { class: "text-base-content", "You own the data. " }
                    "Uninstalling the app or deleting local data deletes the entire library. Backups are your responsibility (subscriptions can be exported as OPML)."
                }
                li {
                    strong { class: "text-base-content", "Feeds refresh only while the app is open. " }
                    "There is no background refresh when the app is closed."
                }
                li {
                    strong { class: "text-base-content", "Episodes download into the app. " }
                    "Playback is set to Stream Only from the built-in server; the separate device-downloads concept goes away."
                }
                li {
                    "Refreshing feeds and downloading episodes uses this device's network and battery."
                }
            }
            p { class: "text-xs text-muted",
                "A dedicated server stays the recommended setup — you can switch later from Settings."
            }
        }
    }
}
