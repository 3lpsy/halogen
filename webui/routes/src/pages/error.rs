//! AppError renders inside Router and keeps the failed URL; navigate away before clearing errors to avoid immediate
//! recurrence. FatalError wraps providers and must use no context, offering web reload or native reopen guidance.
//! Router navigation preserves the native bridge.

use dioxus::prelude::*;

use crate::Route;
use halogen_webui_component_icons::FaceFrown;

#[component]
pub fn AppError(errors: ErrorContext) -> Element {
    let nav = use_navigator();
    // The fallback renders IN PLACE, so the crashed route is the current one. "Go to Home" must not navigate straight
    // back into it: `Home` immediately redirects to `Queue`, so a crash on Queue (or Home itself) made the recovery
    // button re-enter the crashed subtree and bounce right back here. For those, recover to Settings instead, a light,
    // self-contained page.
    let crashed: Route = use_route();
    let home_is_crashed = matches!(crashed, Route::Home {} | Route::Queue {});
    let (target, target_label) = if home_is_crashed {
        (Route::Settings {}, "Go to Settings")
    } else {
        (Route::Home {}, "Go to Home")
    };
    rsx! {
        div { class: "min-h-screen bg-base-100 text-base-content flex items-center justify-center p-4",
            div { class: "max-w-md w-full text-center",
                FaceFrown { class: "w-16 h-16 mx-auto text-muted mb-4" }
                h1 { class: "text-xl font-semibold mb-2", "Something went wrong" }
                p { class: "text-sm text-muted mb-6",
                    "The app hit an unexpected error. Going back usually fixes it; if it keeps happening, clearing local data on this device may help."
                }

                button {
                    class: "btn btn-primary w-full",
                    onclick: {
                        let errors = errors.clone();
                        let target = target.clone();
                        move |_| {
                            // `replace`, not `push`: drop the crashed route from
                            // history so Back doesn't return to (and re-crash) it.
                            nav.replace(target.clone());
                            errors.clear_errors();
                        }
                    },
                    "{target_label}"
                }

                div { class: "divider text-xs text-muted my-6", "Still stuck?" }

                p { class: "text-sm text-muted mb-3",
                    "If the problem persists, removing local data on this device often resolves it."
                }
                button {
                    class: "btn btn-ghost btn-sm w-full",
                    onclick: {
                        let errors = errors.clone();
                        move |_| {
                            nav.replace(Route::CacheControl {});
                            errors.clear_errors();
                        }
                    },
                    "Local data & cache controls"
                }
            }
        }
    }
}

/// The outer, true-root boundary fallback (see [`crate::app::App`]). Renders
/// OUTSIDE `AppProviders` + the `Router`, so it touches **no** context. There's
/// nothing to recover to from here — the providers are what failed — so it just
/// shows the error and offers a restart.
#[component]
pub fn FatalError(errors: ErrorContext) -> Element {
    rsx! {
        div { class: "min-h-screen bg-base-100 text-base-content flex items-center justify-center p-4",
            div { class: "max-w-md w-full text-center",
                FaceFrown { class: "w-16 h-16 mx-auto text-muted mb-4" }
                h1 { class: "text-xl font-semibold mb-2", "The app couldn't start" }
                p { class: "text-sm text-muted mb-4",
                    "An unexpected error stopped the app from loading. Restarting usually fixes it."
                }
                if let Some(err) = errors.error() {
                    pre { class: "text-left text-xs text-error bg-base-200 rounded p-2 mb-4 max-h-32 overflow-auto whitespace-pre-wrap break-words",
                        "{err}"
                    }
                }
                RestartAction {}
            }
        }
    }
}

/// Web: a reload button (a full document reload re-boots the app cleanly).
#[cfg(target_arch = "wasm32")]
#[component]
fn RestartAction() -> Element {
    rsx! {
        button {
            class: "btn btn-primary w-full",
            onclick: move |_| {
                if let Some(win) = web_sys::window() {
                    let _ = win.location().reload();
                }
            },
            "Reload the app"
        }
    }
}

/// Native (mobile/desktop webview): a webview app can't reliably restart itself,
/// so prompt the user to do it.
#[cfg(not(target_arch = "wasm32"))]
#[component]
fn RestartAction() -> Element {
    rsx! {
        p { class: "text-sm text-muted", "Please fully close and reopen the app." }
    }
}
