//! `/cache-control` — the local-data & cache control panel, and (eventually) the
//! app's failure / recovery page.
//!
//! Deliberately standalone: it sits **outside** `RootGuard` and `AppLayout` (no
//! navbar, renders even signed-out) and clears storage by talking to the browser
//! directly via [`cache_purge`] — never the sync worker — so it still works when
//! the data layer is wedged. It must not fail: it pulls in no app state beyond
//! the always-mounted account registry signal (needed by the embedded-library
//! delete, which must also drop the affected accounts), the router (for the
//! close button) and a startup-captured `?message=` param.
//!
//! UX rules: every action confirms inline; actions never navigate away (the only
//! way out is the ✕ → Home); feedback is a local `message` banner (no toast
//! system), plus the `query_message` banner for a `?message=` handed in by a
//! future failure redirect. Asset/service-worker actions reload the page (staying
//! on this route) to take effect.

use dioxus::prelude::*;

use crate::Route;
use halogen_ui_accounts::accounts::AccountsStore;
use halogen_ui_cache_purge as cache_purge;
use halogen_ui_config::ClientConfigStore;
use halogen_ui_icons::XMark;
use halogen_ui_state::embedded;
use halogen_ui_state::hooks::{read_initial_query_param, use_accounts};

#[component]
pub fn CacheControl() -> Element {
    let nav = use_navigator();
    let accounts = use_accounts();
    // Offer the embedded-library delete when this build can run one AND there is
    // anything to delete (a library on disk, or registered embedded accounts).
    let show_embedded_card = embedded::available()
        && (embedded::library_exists()
            || accounts.peek().users.iter().any(|u| u.kind.is_embedded()));
    // Static: the `?message=` captured at startup (the live query is stripped by
    // the router; startup re-capture survives the reload our actions trigger).
    let query_message = read_initial_query_param("message");
    // Reactive: set by an action's outcome, shown at the top.
    let message = use_signal(|| Option::<String>::None);

    rsx! {
        div { class: "min-h-screen bg-base-100 text-base-content",
            div { class: "max-w-xl mx-auto p-4 pt-6",
                div { class: "flex items-center justify-between mb-3",
                    h1 { class: "text-2xl font-bold", "Local data & cache" }
                    button {
                        "aria-label": "Close",
                        class: "btn btn-ghost btn-sm btn-circle",
                        onclick: move |_| {
                            // `replace`: this is a leaf "close" — don't leave the
                            // cache page (or a route that redirected here on failure)
                            // sitting in history for Back to return to.
                            nav.replace(Route::Home {});
                        },
                        XMark { class: "w-5 h-5" }
                    }
                }
                p { class: "text-sm text-muted mb-4",
                    "Trim or reset data stored on this device. Everything here is device-wide and asks for confirmation. Nothing here signs in or contacts the server unless noted."
                }

                // Message handed in via the URL (e.g. by a future failure redirect).
                if let Some(m) = query_message {
                    div { role: "alert", class: "alert mb-3",
                        span { class: "text-sm", "{m}" }
                    }
                }
                // Outcome of the last action on this page.
                if let Some(m) = message() {
                    div { role: "alert", class: "alert alert-success mb-3",
                        span { class: "text-sm", "{m}" }
                    }
                }

                div { class: "space-y-3",
                    ActionCard {
                        title: "Clear content cache",
                        description: "Drops downloaded lists of podcasts, episodes, playlists and play progress. They re-fetch from the server on next use. Safe: keeps you signed in and doesn't touch anything waiting to sync. Reload to refresh what's on screen.",
                        button_label: "Clear content cache",
                        danger: false,
                        on_confirm: move |_| {
                            let mut message = message;
                            spawn(async move {
                                let n = cache_purge::clear_content().await;
                                message.set(Some(cleared_msg("content cache", n)));
                            });
                        },
                    }
                    ActionCard {
                        title: "Clear pending sync queue",
                        description: "Discards offline actions that haven't reached the server yet (subscribes, played marks, reorders…). Use only if syncing is stuck — anything not yet synced is permanently lost.",
                        button_label: "Clear sync queue",
                        danger: true,
                        on_confirm: move |_| {
                            let mut message = message;
                            spawn(async move {
                                let n = cache_purge::clear_outbox().await;
                                message.set(Some(cleared_msg("pending sync queue", n)));
                            });
                        },
                    }
                    ActionCard {
                        title: "Reset view settings",
                        description: "Restores every list's sort and filter choices to their defaults.",
                        button_label: "Reset view settings",
                        danger: false,
                        on_confirm: move |_| {
                            let mut message = message;
                            spawn(async move {
                                let n = cache_purge::clear_view_settings().await;
                                message.set(Some(cleared_msg("view settings", n)));
                            });
                        },
                    }
                    ActionCard {
                        title: "Delete downloaded audio",
                        description: "Removes every episode audio file downloaded to this device — usually the biggest space saver. Streaming and re-downloading still work.",
                        button_label: "Delete downloads",
                        danger: true,
                        on_confirm: move |_| {
                            let mut message = message;
                            spawn(async move {
                                let ok = cache_purge::clear_audio().await;
                                message.set(Some(if ok {
                                    "Deleted downloaded audio.".into()
                                } else {
                                    "No downloaded audio to delete.".into()
                                }));
                            });
                        },
                    }
                    if show_embedded_card {
                        ActionCard {
                            title: "Delete embedded server library",
                            description: "Stops the built-in server and permanently deletes its entire library — subscriptions, episodes, downloads and history stored on this device. This is the only copy; consider exporting OPML first.",
                            button_label: "Delete embedded library",
                            danger: true,
                            on_confirm: move |_| {
                                let mut message = message;
                                let mut accounts = accounts;
                                spawn(async move {
                                    // Each embedded account's client-side namespace first…
                                    let reg_now = accounts.peek().clone();
                                    for u in reg_now.users.iter().filter(|u| u.kind.is_embedded()) {
                                        ClientConfigStore::clear_for(u.key()).await;
                                    }
                                    // …then the server data itself (stops the server first).
                                    if let Err(e) = embedded::destroy().await {
                                        message.set(Some(format!(
                                            "Failed to delete embedded library: {e}"
                                        )));
                                        return;
                                    }
                                    let was_active =
                                        reg_now.active_key().is_some_and(|k| k.is_embedded());
                                    let mut reg = reg_now;
                                    reg.users.retain(|u| !u.kind.is_embedded());
                                    if was_active {
                                        // Mirror ordinary sign-out: a surviving (remote)
                                        // account becomes active rather than dumping the
                                        // user to first-boot setup. Its stored token may
                                        // be stale — the worker's auth-expiry path and
                                        // RootGuard handle that as a normal re-login.
                                        reg.set_active(reg.users.first().map(|u| u.key()));
                                    }
                                    AccountsStore::save(&reg).await;
                                    if was_active && reg.active_key().is_none() {
                                        // Truly the last account — back to first-boot
                                        // setup (the one action here that must navigate).
                                        let _ = navigator().replace("/auth/login");
                                    } else {
                                        message.set(Some(
                                            "Deleted the embedded server library.".into(),
                                        ));
                                    }
                                    accounts.set(reg); // last: may remount the data subtree
                                });
                            },
                        }
                    }
                    ActionCard {
                        title: "Clear device logs",
                        description: "Discards diagnostic logs captured on this device.",
                        button_label: "Clear logs",
                        danger: false,
                        on_confirm: move |_| {
                            let mut message = message;
                            spawn(async move {
                                cache_purge::clear_logs().await;
                                message.set(Some("Cleared device logs.".into()));
                            });
                        },
                    }
                    ActionCard {
                        title: "Reload with fresh app code",
                        description: "Clears cached app assets (JavaScript, WebAssembly, CSS), restarts the service worker, and reloads. Use if the app is stuck on an old or broken version. This reloads the page.",
                        button_label: "Reload fresh",
                        danger: false,
                        on_confirm: move |_| {
                            spawn(async move {
                                cache_purge::clear_cached_assets().await;
                                cache_purge::unregister_service_worker().await;
                                cache_purge::reload_with_message(
                                    "Cleared cached app assets and restarted.",
                                );
                            });
                        },
                    }
                    ActionCard {
                        title: "Clear all saved data & sign out",
                        description: "Wipes all browser storage — every signed-in account, tokens and cached data — then reloads. Keeps downloaded audio. You'll sign in again. (The media cookie is HttpOnly; the server clears it on sign-out.)",
                        button_label: "Clear all & sign out",
                        danger: true,
                        on_confirm: move |_| {
                            spawn(async move {
                                cache_purge::clear_all_storage().await;
                                cache_purge::reload_with_message(
                                    "Signed out and cleared saved data.",
                                );
                                // Native: no reload — land signed out explicitly.
                                #[cfg(not(target_arch = "wasm32"))]
                                {
                                    let mut accounts = accounts;
                                    let _ = navigator().replace("/auth/login");
                                    accounts.set(Default::default());
                                }
                            });
                        },
                    }
                    ActionCard {
                        title: "Delete everything",
                        description: "The full reset: clears all browser storage, downloaded audio, logs and cached app assets, unregisters the service worker, then reloads as a fresh install.",
                        button_label: "Delete everything",
                        danger: true,
                        on_confirm: move |_| {
                            spawn(async move {
                                cache_purge::clear_all_storage().await;
                                cache_purge::clear_audio().await;
                                cache_purge::clear_logs().await;
                                cache_purge::clear_cached_assets().await;
                                cache_purge::unregister_service_worker().await;
                                // Fresh-install semantics: the embedded server
                                // library goes too (no-op error when absent).
                                if embedded::available() {
                                    let _ = embedded::destroy().await;
                                }
                                cache_purge::reload_with_message("Deleted all local data.");
                                // Native: no reload happens, so land the app in a real
                                // first-boot state ourselves (the purge above removed the
                                // on-disk registry + account configs).
                                #[cfg(not(target_arch = "wasm32"))]
                                {
                                    let mut accounts = accounts;
                                    let _ = navigator().replace("/auth/login");
                                    accounts.set(Default::default());
                                }
                            });
                        },
                    }
                }
            }
        }
    }
}

/// Standard "cleared X" / "nothing to clear" message for the count-returning
/// localStorage purges.
fn cleared_msg(what: &str, removed: usize) -> String {
    if removed == 0 {
        format!("No {what} to clear.")
    } else {
        format!("Cleared {what}. Reload to refresh what's on screen.")
    }
}

/// One labelled action with a concise description and an inline two-step confirm
/// (so a stray tap can't wipe anything). `on_confirm` fires only after Confirm.
#[component]
fn ActionCard(
    title: String,
    description: String,
    button_label: String,
    danger: bool,
    on_confirm: EventHandler<()>,
) -> Element {
    let mut confirming = use_signal(|| false);
    let btn_class = if danger {
        "btn btn-sm btn-error"
    } else {
        "btn btn-sm btn-primary"
    };
    rsx! {
        div { class: "card bg-base-200 p-3",
            h2 { class: "font-semibold", "{title}" }
            p { class: "text-sm text-muted mt-1 mb-3", "{description}" }
            if confirming() {
                div { class: "flex items-center gap-2 flex-wrap",
                    span { class: "text-sm font-medium", "Are you sure?" }
                    button {
                        class: "{btn_class}",
                        onclick: move |_| {
                            confirming.set(false);
                            on_confirm.call(());
                        },
                        "Confirm"
                    }
                    button {
                        class: "btn btn-sm btn-ghost",
                        onclick: move |_| confirming.set(false),
                        "Cancel"
                    }
                }
            } else {
                button {
                    class: "{btn_class}",
                    onclick: move |_| confirming.set(true),
                    "{button_label}"
                }
            }
        }
    }
}
