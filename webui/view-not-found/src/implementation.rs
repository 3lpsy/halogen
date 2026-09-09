//! Render unmatched routes outside auth/layout guards using only router context. Offer Home and cache recovery even
//! while signed out or when the data layer fails.

use dioxus::prelude::*;

use halogen_webui_component_icons::FaceFrown;

#[component]
pub fn NotFound() -> Element {
    rsx! {
        div { class: "min-h-screen bg-base-100 text-base-content flex items-center justify-center p-4",
            div { class: "max-w-md w-full text-center",
                FaceFrown { class: "w-16 h-16 mx-auto text-muted mb-4" }
                p { class: "text-5xl font-bold leading-none mb-2", "404" }
                h1 { class: "text-xl font-semibold mb-2", "Page not found" }
                p { class: "text-sm text-muted mb-6",
                    "We couldn't find the page you were looking for. The link may be broken, or the page may have moved."
                }

                Link {
                    to: "/",
                    class: "btn btn-primary w-full",
                    "Go to Home"
                }

                div { class: "divider text-xs text-muted my-6", "Still stuck?" }

                p { class: "text-sm text-muted mb-3",
                    "If you believe you reached this page because of an error, removing local data on this device may fix it."
                }
                Link {
                    to: "/cache-control",
                    class: "btn btn-ghost btn-sm w-full",
                    "Local data & cache controls"
                }
            }
        }
    }
}
