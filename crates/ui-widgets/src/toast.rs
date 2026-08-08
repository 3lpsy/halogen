//! Renders the global toast queue. Mounted once at the app root so it overlays
//! every route (including the auth screens, which sit outside `AppLayout`).

use dioxus::prelude::*;

use halogen_ui_icons::XMark;
use halogen_ui_platform::time::sleep_ms;
use halogen_ui_state::hooks::use_toasts;
use halogen_ui_toast::Toast;

/// Stacked toast overlay (daisyUI `toast` + `alert`). High z-index so it sits
/// above the navbar/dock (z-50) and players.
#[component]
pub fn ToastContainer() -> Element {
    let toasts = use_toasts();
    let items = toasts().toasts.clone();

    rsx! {
        // mt-[…]: keep toasts below the iOS notch/status bar in an installed
        // PWA (daisyUI `toast-top` pins to the viewport top edge; the inset is
        // 0 in a normal browser tab). `toast-center` centers the stack
        // horizontally at the top of the screen.
        div { class: "toast toast-top toast-center z-[100] mt-[env(safe-area-inset-top)]",
            for toast in items {
                ToastItem { key: "{toast.id}", toast }
            }
        }
    }
}

/// A single toast. Owns its own auto-dismiss timer (cancelled on unmount, which
/// happens on manual dismiss or when a dedup-refresh replaces it).
#[component]
fn ToastItem(toast: Toast) -> Element {
    let store = use_toasts();
    let id = toast.id;
    let timeout_ms = toast.timeout_ms;
    let alert_class = toast.level.alert_class();
    let message = toast.message.clone();

    use_future(move || async move {
        let mut store = store;
        if let Some(ms) = timeout_ms {
            sleep_ms(ms).await;
            store.write().dismiss(id);
        }
    });

    rsx! {
        div { class: "alert {alert_class} shadow-lg",
            span { "{message}" }
            button {
                class: "btn btn-ghost btn-circle",
                onclick: move |_| {
                    let mut store = store;
                    store.write().dismiss(id);
                },
                XMark { class: "w-4 h-4" }
            }
        }
    }
}
