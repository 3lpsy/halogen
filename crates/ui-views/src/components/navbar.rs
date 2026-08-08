use dioxus::prelude::*;

use crate::Route;
use halogen_ui_appstate::ConnectionHealth;
use halogen_ui_config::AccountKey;
use halogen_ui_config::ClientConfigStore;
use halogen_ui_icons::{GlobeAlt, User};
use halogen_ui_state::commands;
use halogen_ui_state::embedded_session::{sign_out_account_smart, switch_account_smart};
use halogen_ui_state::hooks::{
    use_accounts, use_config, use_connection_health, use_dispatch, use_toast,
};

/// Navbar component displayed at the top of the app.
///
/// Shows "halogen" and a clickable status indicator that doubles as the
/// "Go Offline" / "Go Online" toggle. The globe icon has five states:
/// - **Connecting** — neutral (cold start, reachability not yet determined).
/// - **Online** — green.
/// - **Degraded** — yellow (reachable but high latency; still online).
/// - **Manually offline** — neutral, label "Offline*" (the asterisk marks a
///   *chosen* offline, distinct from a real outage).
/// - **Offline** — neutral, label "Offline" (can't reach the server).
///
/// Manual offline wins over real connectivity and is persisted in `ClientConfig`
/// (survives reload); toggling it pushes `SetOffline` to the worker, which tears
/// down / rebuilds the connectivity WebSocket accordingly.
#[component]
pub fn Navbar() -> Element {
    let mut config = use_config();
    let dispatch = use_dispatch();
    // Memo slice: re-renders only when the connection tier (or smoothed RTT)
    // changes, not on every worker publish.
    let connection = use_connection_health();

    // Manual offline wins over real connectivity: when it's on we always present a
    // chosen-offline state regardless of reachability.
    let manual_offline = config.read().manual_offline;
    // Embedded server: "Go Offline" would sever the app from itself (the config
    // overlay force-clears a persisted manual_offline too) — the status stays a
    // pure indicator of the in-process server's health.
    let embedded_mode = config.read().server_kind.is_embedded();
    // (icon colour class, label). The class is a `&'static str` (the icon prop
    // requires it — no runtime format string).
    let (icon_class, label) = if manual_offline {
        ("w-5 h-5 text-base-content", "Offline*")
    } else {
        match connection() {
            ConnectionHealth::Unknown => ("w-5 h-5 text-base-content", "Connecting"),
            ConnectionHealth::Online { .. } => ("w-5 h-5 text-success", "Online"),
            ConnectionHealth::Degraded { .. } => ("w-5 h-5 text-warning", "Degraded"),
            ConnectionHealth::Offline => ("w-5 h-5 text-base-content", "Offline"),
        }
    };
    let action = if embedded_mode {
        "Connection status"
    } else if manual_offline {
        "Go online"
    } else {
        "Go offline"
    };

    // Toggle + persist + push to the worker. Inert in embedded mode (indicator only).
    let on_toggle = move |_| {
        if embedded_mode {
            return;
        }
        let now = !config.peek().manual_offline;
        config.write().manual_offline = now;
        commands::set_offline(&dispatch, now);
        let snapshot = config.peek().clone();
        spawn(async move {
            ClientConfigStore::save(&snapshot).await;
        });
    };

    rsx! {
        div {
            // min-h-12 (3rem): daisyUI's `.navbar` defaults to `min-height: 4rem`,
            // but AppLayout/Sidebar reserve only `3rem + inset` for this bar — a
            // taller bar would overlap the top of the page content (the
            // filter/sort/search controls bar tucks under it). The `btn-sm` brand
            // button (2rem) fits in 3rem, so pin the bar to the reserved height and
            // the controls bar clears it cleanly. Keep this in lockstep with the
            // `3rem` literals in AppLayout's `pt-` and Sidebar's `top-`.
            //
            // pt-[env(safe-area-inset-top)]: in an installed iOS PWA
            // (black-translucent status bar + viewport-fit=cover) the page
            // extends under the status bar/notch — the bar's background fills
            // that area and the padding keeps its content below it. In a normal
            // browser tab the inset is 0 and this is a no-op. AppLayout/Sidebar
            // both reserve `3rem + env(safe-area-inset-top)` for this bar, so
            // the padding (not a margin/offset) is load-bearing for alignment.
            class: "navbar min-h-12 bg-navbar shadow-md absolute top-0 left-0 right-0 z-50 pb-0 pt-[env(safe-area-inset-top)]",
            div {
                class: "flex-1",
                Link {
                    to: Route::Home {},
                    class: "btn btn-sm btn-ghost text-lg normal-case",
                    "halogen"
                }
            }
            div {
                class: "flex-none flex items-center gap-1",
                // Clickable status = the Go Offline / Go Online toggle. Icon carries
                // the green/white state; the label text colour is constant.
                button {
                    class: "btn btn-ghost btn-sm gap-2 normal-case",
                    "aria-label": "{action}",
                    title: "{action}",
                    onclick: on_toggle,
                    GlobeAlt { class: icon_class }
                    // Fixed width + centered: the label cycles through
                    // Online/Degraded/Offline*/Offline as connectivity resolves
                    // post-hydration. A content-sized span would resize the
                    // flex-none cluster on each transition (a measured CLS
                    // culprit); pin it to fit the widest label ("Degraded").
                    span {
                        class: "text-xs text-base-content w-14 text-center inline-block",
                        "{label}"
                    }
                }
                // Account switcher (only shown once at least one user is signed in).
                UserMenu {}
            }
        }
    }
}

/// Navbar account switcher: a dropdown listing every signed-in account with a
/// one-tap switch, plus "Add account" and "Sign out". Hidden when signed out.
///
/// The navigator is read lazily via `navigator()` inside the click handlers (not
/// the `use_navigator` hook) so the component renders without a router in the
/// smoke test — handlers only run when a router is present.
#[component]
fn UserMenu() -> Element {
    let accounts = use_accounts();
    let toast = use_toast();
    let dispatch = use_dispatch();

    let accts = accounts.read();
    let Some(active_key) = accts.active_key() else {
        return rsx! {};
    };
    let active_name = accts
        .users
        .iter()
        .find(|u| u.key() == active_key)
        .map(|u| u.username.clone())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "Account".to_string());
    // (key, label, dom key) — the segment doubles as the unique DOM key, since
    // ids alone can repeat across server kinds.
    let others: Vec<(AccountKey, String, String)> = accts
        .users
        .iter()
        .filter(|u| u.key() != active_key)
        .map(|u| {
            let label = if u.username.is_empty() {
                format!("User {}", u.id)
            } else {
                u.username.clone()
            };
            (u.key(), label, u.key().segment())
        })
        .collect();
    let active_embedded = active_key.is_embedded();
    drop(accts);

    rsx! {
        div { class: "dropdown dropdown-end",
            div {
                tabindex: "0",
                role: "button",
                class: "btn btn-ghost btn-sm",
                "aria-label": "Accounts",
                title: "Accounts",
                User { class: "w-5 h-5 text-base-content" }
            }
            ul {
                tabindex: "0",
                class: "menu dropdown-content bg-base-200 rounded-box z-[60] mt-2 w-56 p-2 shadow",
                li { class: "menu-title",
                    "{active_name}"
                    if active_embedded {
                        span { class: "badge badge-ghost badge-xs ml-1", "this device" }
                    }
                }
                for (uid, uname, dom_key) in others {
                    li {
                        key: "{dom_key}",
                        button {
                            onclick: move |_| {
                                let accounts = accounts;
                                let toast = toast;
                                spawn(async move { switch_account_smart(accounts, toast, uid).await });
                            },
                            User { class: "w-4 h-4" }
                            "Switch to {uname}"
                            if uid.is_embedded() {
                                span { class: "badge badge-ghost badge-xs", "this device" }
                            }
                        }
                    }
                }
                li {
                    button {
                        onclick: move |_| {
                            // Embedded: create a user on the in-process server;
                            // remote: the ordinary login flow.
                            if accounts.peek().active_key().is_some_and(|k| k.is_embedded()) {
                                let _ = navigator().push(Route::AddEmbeddedUser {});
                            } else {
                                let _ = navigator().push(Route::Login {});
                            }
                        },
                        "Add account"
                    }
                }
                li {
                    button {
                        class: "text-error",
                        onclick: move |_| {
                            let accounts = accounts;
                            let dispatch = dispatch;
                            spawn(async move {
                                // Copy the id out so the `peek()` read guard drops
                                // before the await: `sign_out_account` calls
                                // `accounts.set(...)`, which would otherwise panic
                                // with BorrowMutError while the read borrow is held.
                                let active = accounts.peek().active_key();
                                if let Some(key) = active {
                                    sign_out_account_smart(accounts, &dispatch, key).await;
                                }
                            });
                        },
                        "Sign out"
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::channel::mpsc::UnboundedReceiver;
    use halogen_ui_accounts::accounts::Accounts;
    use halogen_ui_appstate::EpisodeState;
    use halogen_ui_commands::Command;
    use halogen_ui_config::ClientConfig;
    use halogen_ui_toast::ToastQueue;

    // Navbar reads several contexts (EpisodeState, ClientConfig, dispatch coroutine,
    // the account registry, and the toast queue), so the smoke test wraps it in a
    // provider harness mirroring the real providers. With a default (signed-out)
    // registry the `UserMenu` renders nothing, so no router context is needed.
    #[test]
    fn navbar_component_creates() {
        fn harness() -> Element {
            let app_state = use_signal(EpisodeState::default);
            use_context_provider(|| app_state);
            let config = use_signal(ClientConfig::default);
            use_context_provider(|| config);
            let accounts = use_signal(Accounts::default);
            use_context_provider(|| accounts);
            let toasts = use_signal(ToastQueue::default);
            use_context_provider(|| toasts);
            let dispatch = use_coroutine(|_rx: UnboundedReceiver<Command>| async {});
            use_context_provider(|| dispatch);
            rsx! { Navbar {} }
        }
        let mut vdom = VirtualDom::new(harness);
        vdom.rebuild(&mut dioxus::core::NoOpMutations);
    }
}
