use dioxus::prelude::*;

use crate::components::{nav_icon, nav_items};
use halogen_ui_state::hooks::{use_config, use_is_admin};

/// Mobile-only navigation page. The dock shows the first five nav items plus a
/// "More" slot that links here; this page lists **every** nav destination (the
/// same source of truth the sidebar renders on desktop).
#[component]
pub fn Menu() -> Element {
    let config = use_config();
    let is_admin = use_is_admin();
    let nav = nav_items(&config(), is_admin());

    rsx! {
        div { class: "p-4",
            h1 { class: "text-2xl font-bold mb-4", "Menu" }
            nav { class: "flex flex-col gap-1",
                for item in nav {
                    Link {
                        key: "{item.label}",
                        to: item.route.clone(),
                        class: "flex items-center gap-3 px-3 py-3 rounded-lg text-foreground hover:bg-sidebar-hover transition-colors",
                        {nav_icon(&item.key, "w-5 h-5")}
                        span { class: "font-medium", "{item.label}" }
                    }
                }
            }
        }
    }
}
