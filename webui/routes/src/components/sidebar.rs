use dioxus::prelude::*;

use crate::components::nav::{nav_icon, nav_items, use_active_nav_key};
use halogen_webui_hooks::{use_config, use_is_admin};

/// Sidebar component for desktop view.
/// Displays navigation items from a single source of truth.
#[component]
pub fn Sidebar() -> Element {
    let config = use_config();
    let is_admin = use_is_admin();
    // Active highlight is resolved by section (detail/form pages fold into their
    // parent nav item; an episode page inherits its origin) — see `use_active_nav_key`.
    let active_key = use_active_nav_key();
    let nav = nav_items(&config(), is_admin());
    let nav_with_active: Vec<_> = nav
        .into_iter()
        .map(|item| {
            let active = active_key.as_ref() == Some(&item.key);
            let cls = format!(
                "flex items-center gap-3 px-3 py-2 rounded-lg text-foreground hover:bg-sidebar-hover transition-colors {}",
                if active { "bg-sidebar-hover" } else { "" }
            );
            (item, cls)
        })
        .collect();

    rsx! {
        div {
            class: "sidebar absolute left-0 top-[calc(3rem+env(safe-area-inset-top))] bottom-0 w-64 bg-sidebar overflow-y-auto z-40",
            nav {
                class: "flex flex-col p-4 space-y-1",
                for (item, cls) in nav_with_active {
                    Link {
                        key: "{item.key:?}",
                        to: item.route.clone(),
                        class: cls,
                        {nav_icon(&item.key, "w-5 h-5")}
                        span {
                            class: "font-medium",
                            "{item.label}"
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidebar_component_creates() {
        let mut vdom = VirtualDom::new(|| rsx! { Sidebar {} });
        vdom.rebuild(&mut dioxus::core::NoOpMutations);
    }
}
