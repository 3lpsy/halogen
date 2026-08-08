//! Shared shell for the unauthenticated screens (Login + EmbeddedServerSetup).
//!
//! Both are a centered "Halogen" card: a title + subtitle over a `bg-sidebar`
//! box holding a `space-y-4` form, an inline error line, and a full-width submit
//! button whose label flips while a request is in flight. Only the inputs and the
//! submit handler differ, so those are the caller's (`children` + `onsubmit`).

use dioxus::prelude::*;

use halogen_ui_icons::ChevronLeft;

/// Centered auth card. `children` are the form inputs; the error line + submit
/// button are rendered here.
#[component]
pub fn AuthCard(
    /// Sub-heading under the "Halogen" title.
    subtitle: String,
    /// Tailwind max-width for the card.
    max_width: String,
    /// Inline error to show above the submit button, if any.
    error: ReadSignal<Option<String>>,
    /// Disables the button and swaps in `pending_label` while a request runs.
    loading: ReadSignal<bool>,
    /// Submit button label when idle.
    submit_label: String,
    /// Submit button label while `loading`.
    pending_label: String,
    onsubmit: EventHandler<FormEvent>,
    /// Optional back action, rendered as a chevron button at the card's top
    /// left (embedded setup → back to Login). `None` (Login — the flow's
    /// first screen) renders nothing.
    #[props(default)]
    on_back: Option<EventHandler<MouseEvent>>,
    /// Optional content rendered inside the card BELOW the submit button
    /// (secondary flows, e.g. Login's "Use Embedded Server" link).
    #[props(default)]
    footer: Option<Element>,
    children: Element,
) -> Element {
    rsx! {
        div { class: "min-h-screen flex items-center justify-center bg-background p-4",
            div { class: "w-full {max_width}",
                if let Some(back) = on_back {
                    button {
                        class: "btn btn-ghost btn-sm gap-1 pl-0 mb-2",
                        "aria-label": "Back",
                        onclick: move |e| back.call(e),
                        ChevronLeft { class: "w-4 h-4" }
                        "Back"
                    }
                }
                div { class: "text-center mb-8",
                    h1 { class: "text-3xl font-bold", "Halogen" }
                    p { class: "text-muted mt-2", "{subtitle}" }
                }
                div { class: "bg-sidebar p-8 rounded-lg",
                    form {
                        class: "space-y-4",
                        onsubmit: move |e| onsubmit.call(e),
                        {children}
                        if let Some(err_msg) = error() {
                            div { class: "text-error text-sm text-center", "{err_msg}" }
                        }
                        button {
                            class: "btn btn-primary w-full",
                            r#type: "submit",
                            disabled: "{loading()}",
                            if loading() {
                                "{pending_label}"
                            } else {
                                "{submit_label}"
                            }
                        }
                    }
                    if let Some(footer) = footer {
                        {footer}
                    }
                }
            }
        }
    }
}
