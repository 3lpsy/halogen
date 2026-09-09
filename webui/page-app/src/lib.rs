use dioxus::prelude::*;

use halogen_webui_component_widgets::{Confirm, ConfirmHost, QuickContextMenuHost, QuickMenu};
use halogen_webui_hooks::{use_config, use_episodes, use_now_playing};
use halogen_webui_player_controls::{MiniPlayer, NowPlayingScreen};

/// Keep navbar, sidebar, dock, and player absolute within one `100dvh` root; fixed bars drift in installed iOS PWAs.
/// Main owns scrolling and safe-area/player padding. Root-level quick menus escape row clipping.
pub fn use_app_layout<R: Routable>(navbar: Element, sidebar: Element, dock: Element) -> Element {
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
    use_context_provider(|| Signal::new(halogen_webui_hooks::ScrollMemory::default()));
    // Instant-nav companion to ScrollMemory: last committed rows per list, painted
    // on frame one by `use_row_memory`; cleared with this layout on account switch.
    use_context_provider(|| Signal::new(halogen_webui_hooks::RowMemory::default()));

    // Apply font scaling to `<html>` because Tailwind/daisyUI sizes use rem. Mirror the fixed percent string to
    // localStorage so the next pre-paint script avoids a full-layout shift; fixed values are safe to interpolate.
    let config = use_config();
    use_effect(move || {
        let pct = config().font_size.percent();
        document::eval(&format!(
            "document.documentElement.style.fontSize = '{pct}'; \
             try {{ localStorage.setItem('halogen.fontsize', '{pct}'); }} catch (e) {{}}"
        ));
    });

    // Reserve 4rem for each visible dock/player using literal Tailwind classes. Memoize only player presence so its
    // 250ms position updates do not rerender the shell.
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
            {navbar}
            div {
                class: "hidden md:block",
                {sidebar}
            }
            main {
                id: "scroll-container",
                class: "{main_class}",
                // iOS momentum scrolling — not a Tailwind utility, so it must be a
                // real CSS declaration (an inline style), not a class.
                style: "-webkit-overflow-scrolling: touch",
                Outlet::<R> {}
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
                {dock}
            }
            QuickContextMenuHost {}
            ConfirmHost {}
        }
    }
}
