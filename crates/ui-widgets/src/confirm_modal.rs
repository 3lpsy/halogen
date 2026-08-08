//! Shared confirmation dialog (daisyUI modal).
//!
//! One modal shell for every "are you sure?" prompt — destructive purges,
//! podcast delete, server restart, config save/clear, and external-link opens.
//! Render it conditionally (the caller owns the "is it open?" signal):
//!
//! ```ignore
//! if confirm_restart() {
//!     ConfirmModal {
//!         title: "Restart the server?",
//!         body: "…",
//!         confirm_label: "Restart",
//!         busy: restarting(),
//!         on_cancel: move |_| confirm_restart.set(false),
//!         on_confirm: move |_| { /* spawn the action */ },
//!     }
//! }
//! ```

use dioxus::prelude::*;

/// A confirm/cancel dialog. The caller controls visibility (render it only when
/// open) and owns both callbacks; this component only draws the shell.
#[component]
pub fn ConfirmModal(
    /// Heading text.
    title: String,
    /// Primary body copy.
    body: String,
    /// Optional smaller muted line beneath the body (e.g. the external URL note).
    #[props(default)]
    note: Option<String>,
    /// Confirm button/anchor label.
    confirm_label: String,
    /// Cancel button label.
    #[props(default = "Cancel".to_string())]
    cancel_label: String,
    /// Style the confirm action as destructive (`btn-error`) instead of primary.
    #[props(default)]
    danger: bool,
    /// Disable both controls and show a spinner on confirm (action in flight).
    #[props(default)]
    busy: bool,
    /// When set, the confirm control is a real anchor opening in a new tab
    /// (`noopener`) rather than a button — for "open external link" confirms.
    #[props(default)]
    confirm_href: Option<String>,
    /// `aria-labelledby` target id (override when two confirm modals can render
    /// in the same subtree; defaults to a shared id, fine when only one shows).
    #[props(default = "confirm-modal-title".to_string())]
    title_id: String,
    on_cancel: EventHandler<()>,
    on_confirm: EventHandler<()>,
) -> Element {
    let confirm_class = if danger {
        "btn btn-error"
    } else {
        "btn btn-primary"
    };
    rsx! {
        div { class: "modal modal-open",
            div {
                class: "modal-box",
                role: "dialog",
                "aria-modal": "true",
                "aria-labelledby": "{title_id}",
                h3 { id: "{title_id}", class: "font-bold text-lg", "{title}" }
                p { class: "py-3 text-sm break-all text-base-content/80", "{body}" }
                if let Some(note) = note {
                    p { class: "text-xs text-muted", "{note}" }
                }
                div { class: "modal-action",
                    button {
                        class: "btn btn-ghost",
                        r#type: "button",
                        disabled: busy,
                        onclick: move |_| on_cancel.call(()),
                        "{cancel_label}"
                    }
                    if let Some(href) = confirm_href {
                        a {
                            class: "{confirm_class}",
                            href: "{href}",
                            target: "_blank",
                            rel: "noopener noreferrer",
                            onclick: move |_| on_confirm.call(()),
                            "{confirm_label}"
                        }
                    } else {
                        button {
                            class: "{confirm_class}",
                            r#type: "button",
                            disabled: busy,
                            onclick: move |_| on_confirm.call(()),
                            if busy {
                                span { class: "loading loading-spinner" }
                            }
                            "{confirm_label}"
                        }
                    }
                }
            }
            // Backdrop dismissal is gated on `busy` like the Cancel button — the
            // dialog must stay up while the confirmed action is in flight.
            div {
                class: "modal-backdrop",
                onclick: move |_| {
                    if !busy {
                        on_cancel.call(())
                    }
                },
            }
        }
    }
}
