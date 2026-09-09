use dioxus::prelude::*;

use crate::Route;
use crate::components::nav::{nav_icon, nav_items, use_active_nav_key};
use halogen_webui_component_icons::Bars;
use halogen_webui_config::FontSize;
use halogen_webui_hooks::{use_config, use_is_admin};

/// Dock component for mobile view. Mirrors the first five sidebar nav items (four at Large/XLarge UI font, so the row
/// doesn't crunch), then a **"More"** slot that links to the mobile-only [`Menu`](crate::pages::menu::Menu) page
/// listing every destination. (Previously this used a bottom-sheet; a dedicated page is simpler and is what
/// `FEATURE_navigation.md` intends.)
#[component]
pub fn Dock() -> Element {
    let config = use_config();
    let is_admin = use_is_admin();
    // The active nav section, resolved by route family + history (see
    // `use_active_nav_key`). A primary slot lights when it owns that section.
    let active_key = use_active_nav_key();
    let nav = nav_items(&config(), is_admin());

    // Large UI font sizes make the labels wide enough to crunch the row, so drop
    // one primary slot (it falls into "More") to give the rest breathing room.
    let primary_count = match config().font_size {
        FontSize::Large | FontSize::XLarge => 4,
        FontSize::Small | FontSize::Medium => 5,
    };

    let primary: Vec<_> = nav
        .iter()
        .take(primary_count)
        .map(|item| {
            let active = active_key.as_ref() == Some(&item.key);
            let cls = format!(
                "flex flex-col items-center justify-center flex-1 py-1 {}",
                if active { "text-accent" } else { "text-muted" }
            );
            (
                item.route.clone(),
                cls,
                item.key.clone(),
                item.label.clone(),
            )
        })
        .collect();

    // "More" lights for anything NOT owned by a primary slot, the menu page itself, plus every destination reached
    // through it (Discover, History, Settings, Polling, their sub-pages, …). Episode pages whose origin was a primary
    // section light that primary instead, so they fall through here only when opened from a non-primary page.
    let more_active = !nav
        .iter()
        .take(primary_count)
        .any(|item| active_key.as_ref() == Some(&item.key));
    let more_cls = format!(
        "flex flex-col items-center justify-center flex-1 py-1 {}",
        if more_active {
            "text-accent"
        } else {
            "text-muted"
        }
    );

    rsx! {
        div {
            // The safe-area padding lives on THIS outer bar (background extends down into the iOS home-indicator area;
            // total height 4rem + inset), not on the inner row: padding inside the fixed-height `h-16` row squeezed the
            // icons upward until they overflowed the bar's top edge in an installed PWA. AppLayout reserves `4rem +
            // env(safe-area-inset-bottom)` and MiniPlayer sits at that same offset, keep all three in sync.
            class: "absolute bottom-0 left-0 right-0 bg-navbar border-t border-border z-50 pb-[env(safe-area-inset-bottom)]",
            div {
                class: "flex justify-around items-center h-16",
                for (route, cls, key, label) in primary {
                    Link {
                        key: "{label}",
                        to: route,
                        class: cls,
                        {nav_icon(&key, "w-6 h-6")}
                        // Smaller label on the narrowest phones (<380px, e.g. SE),
                        // normal `text-xs` from normal phones up.
                        span { class: "text-[10px] min-[380px]:text-xs mt-1", "{label}" }
                    }
                }
                // "More" — always present; links to the full mobile nav page.
                Link {
                    to: Route::Menu {},
                    class: more_cls,
                    Bars { class: "w-6 h-6" }
                    span { class: "text-[10px] min-[380px]:text-xs mt-1", "More" }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dock_component_creates() {
        let mut vdom = VirtualDom::new(|| rsx! { Dock {} });
        vdom.rebuild(&mut dioxus::core::NoOpMutations);
    }

    // NOTE: the dock's "first N nav items + always-present More slot" shape (`.take(primary_count)` + the inline More
    // `Link`) lives only inside the `rsx!` body, not a pure function, so it isn't unit-testable here without a render
    // harness that inspects emitted nodes (none exists in this crate). Skipped per the task's guidance;
    // `dock_component_creates` covers that it mounts.
}
