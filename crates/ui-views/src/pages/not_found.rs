//! `/not-found` — the 404 / bad-link escape hatch.
//!
//! Deliberately standalone, like [`CacheControl`](crate::pages::cache_control): it
//! sits **outside** `RootGuard` and `AppLayout` (no navbar, renders even signed-out
//! or mid-failure) and pulls in **no** app state — only the router — so a bad link
//! always lands somewhere that renders. A catch-all redirect (see the `Route` enum)
//! funnels every otherwise-unmatched path here. From here the user can go Home or,
//! if a wedged local cache caused the bad link, open the local-data controls to
//! recover.

use dioxus::prelude::*;

use crate::Route;
use halogen_ui_icons::FaceFrown;

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
                    to: Route::Home {},
                    class: "btn btn-primary w-full",
                    "Go to Home"
                }

                div { class: "divider text-xs text-muted my-6", "Still stuck?" }

                p { class: "text-sm text-muted mb-3",
                    "If you believe you reached this page because of an error, removing local data on this device may fix it."
                }
                Link {
                    to: Route::CacheControl {},
                    class: "btn btn-ghost btn-sm w-full",
                    "Local data & cache controls"
                }
            }
        }
    }
}
