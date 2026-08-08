use dioxus::prelude::*;

use crate::Route;
use crate::components::{
    Confirm, ConfirmHost, Dock, MiniPlayer, Navbar, NowPlayingScreen, QuickContextMenuHost,
    QuickMenu, Sidebar,
};
use halogen_ui_state::hooks::{use_config, use_episodes, use_now_playing};

/// App layout with locked navbar/dock and a single scrolling content region.
///
/// Shell is a single `relative h-[100dvh]` box (no page scroll); every bar is
/// `absolute` within it, so they all share ONE coordinate space sized to the
/// dynamic viewport. (A `fixed`-bar shell drifts on installed iOS PWAs: the
/// fixed-positioning ICB and this root's box don't coincide under
/// `viewport-fit=cover` + `black-translucent`, so the navbar rode up off the
/// content and the dock stopped short of the visual bottom.)
/// - Navbar: absolute top, z-50
/// - Sidebar: absolute left (desktop only), z-40
/// - Main: the single scroll region with safe-area padding (also reserving room
///   for the dock + mini player — see `main_class` below)
/// - MiniPlayer: bottom bar when something is loaded; expands to the NowPlayingScreen overlay
/// - Dock: absolute bottom (mobile only), z-50
/// - QuickContextMenuHost: the shared right slide-in menu (`use_quick_menu()`)
#[component]
pub fn AppLayout() -> Element {
    // Held only to FORWARD as a prop (MiniPlayer/NowPlayingScreen). The layout never
    // `.read()`s it in this body, so AppLayout is not an EpisodeState subscriber and
    // worker publishes don't re-render the shell.
    let app_state = use_episodes();
    let mut show_now_playing = use_signal(|| false);
    // Shared right slide-in menu, opened from anywhere via `use_quick_menu()`.
    use_context_provider(QuickMenu::new);
    // Shared destructive-confirm modal, opened via `use_confirm()`.
    use_context_provider(Confirm::new);
    // In-memory scroll-position memory for the list pages (no disk). Lives here —
    // the persistent layout wrapping every list route — so it survives navigation
    // between them. Read/written via `use_scroll_memory`.
    use_context_provider(|| Signal::new(halogen_ui_state::hooks::ScrollMemory::default()));
    // Instant-nav companion to ScrollMemory: last committed rows per list, painted
    // on frame one by `use_row_memory`; cleared with this layout on account switch.
    use_context_provider(|| Signal::new(halogen_ui_state::hooks::RowMemory::default()));

    // UI font size scaling. Must land on `<html>`: every Tailwind/daisyUI size
    // (text-sm, p-4, --size-field…) is rem-based, and rem resolves against the
    // root element only — a font-size on any inner div would scale nothing.
    // The effect re-runs whenever the config signal changes.
    //
    // It ALSO mirrors the percent into `localStorage` so the pre-paint script in
    // index.html can apply it on the NEXT cold load before first paint. Without
    // that, this post-mount write was the page's first font-size set, reflowing the
    // whole rem-based layout from the 16px default to e.g. 125% in one big CLS once
    // config resolved. `percent()` is a fixed `&'static str` (e.g. "125%"), so the
    // interpolation is injection-safe.
    let config = use_config();
    use_effect(move || {
        let pct = config().font_size.percent();
        document::eval(&format!(
            "document.documentElement.style.fontSize = '{pct}'; \
             try {{ localStorage.setItem('halogen.fontsize', '{pct}'); }} catch (e) {{}}"
        ));
    });

    // When the mini player is visible it overlays the bottom of the scroll
    // region, so reserve its 4rem too: mobile = dock(4) + mini(4) = 8rem;
    // desktop has no dock but the mini is 4rem. Two literal class strings so
    // Tailwind's source scan emits both `pb-*` variants.
    // Only the *presence* of a now-playing episode matters here (it adds the
    // mini-player's height to the scroll padding). Read it through a memo so the
    // player's ~4×/sec position/buffering ticks — which mutate `now_playing` but
    // not its `is_some()` — don't re-render the whole layout each tick.
    let now_playing = use_now_playing();
    let playing = use_memo(move || now_playing.read().is_some());
    // `overflow-x-hidden` is the app-wide "never scroll horizontally" guard
    // (`overflow-y-auto` alone computes overflow-x to `auto`).
    let main_class = if playing() {
        "absolute inset-0 overflow-y-auto overflow-x-hidden overscroll-contain pt-[calc(3rem+env(safe-area-inset-top))] pb-[calc(8rem+env(safe-area-inset-bottom))] md:pb-16 md:ml-64"
    } else {
        "absolute inset-0 overflow-y-auto overflow-x-hidden overscroll-contain pt-[calc(3rem+env(safe-area-inset-top))] pb-[calc(4rem+env(safe-area-inset-bottom))] md:pb-0 md:ml-64"
    };

    rsx! {
        div {
            class: "relative h-[100dvh] overflow-hidden bg-background",
            Navbar {}
            div {
                class: "hidden md:block",
                Sidebar {}
            }
            main {
                id: "scroll-container",
                class: "{main_class}",
                // iOS momentum scrolling — not a Tailwind utility, so it must be a
                // real CSS declaration (an inline style), not a class.
                style: "-webkit-overflow-scrolling: touch",
                Outlet::<Route> {}
            }
            MiniPlayer {
                app_state,
                on_expand: move |_| show_now_playing.set(true),
            }
            if show_now_playing() {
                NowPlayingScreen {
                    app_state,
                    on_close: move |_| show_now_playing.set(false),
                }
            }
            div {
                class: "md:hidden",
                Dock {}
            }
            QuickContextMenuHost {}
            ConfirmHost {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_layout_component_creates() {
        let mut vdom = VirtualDom::new(|| rsx! { AppLayout {} });
        vdom.rebuild(&mut dioxus::core::NoOpMutations);
    }
}
