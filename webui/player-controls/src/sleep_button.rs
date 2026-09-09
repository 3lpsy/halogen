//! Tap toggles the default sleep timer; hold opens repeatable minute adjustments, dismissed by the backdrop. Explicit
//! open state avoids focus-triggered menus conflicting with taps. The controller owns timer logic; active minutes
//! appear in the badge.

use dioxus::prelude::*;

use halogen_webui_component_icons::Moon;
use halogen_webui_hooks::{use_config, use_player_controller};
use halogen_webui_platform::time::sleep_ms;

/// Hold duration (ms) that distinguishes a long-press (open the menu) from a tap.
const LONG_PRESS_MS: u64 = 450;

#[component]
pub fn SleepTimerButton() -> Element {
    let player = use_player_controller();
    let config = use_config();

    // The controller is built once and never replaced, so peek its sleep signal out.
    let sleep = player.peek().sleep_state();
    // Badge value + active flag via memos so the button only re-renders when the
    // *displayed minute* (or active state) changes — not 4×/sec as the timer ticks.
    let minutes = use_memo(move || sleep.read().remaining_minutes());
    let active = use_memo(move || sleep.read().is_active());
    // Step size for the +/− controls (peeked reactively; cheap memo).
    let inc = use_memo(move || config.read().playback_prefs.sleep_increment_minutes as i64);

    // Menu open state + tap-vs-hold tracking. `press_gen` is bumped on every
    // pointerdown/up/leave so a stale long-press timer can detect it was cancelled.
    let mut open = use_signal(|| false);
    let mut held = use_signal(|| false);
    let mut press_gen = use_signal(|| 0u64);

    let on_pointer_down = move |_| {
        held.set(false);
        let generation = *press_gen.peek() + 1;
        press_gen.set(generation);
        spawn(async move {
            sleep_ms(LONG_PRESS_MS as u32).await;
            // Still the same press (not released/cancelled) → it's a hold: open the menu.
            if *press_gen.peek() == generation {
                held.set(true);
                open.set(true);
            }
        });
    };
    let on_pointer_up = move |_| {
        let was_hold = *held.peek();
        // Cancel any pending long-press timer for this press. (Read into a local
        // first: the peek guard would otherwise overlap the `set` mutable borrow.)
        let next = *press_gen.peek() + 1;
        press_gen.set(next);
        // A plain tap (not a hold) toggles the timer.
        if !was_hold {
            player.peek().toggle_sleep();
        }
    };
    // Moving/leaving/cancelling mid-press aborts the pending long-press by bumping
    // the generation so the spawned timer no-ops (inlined per handler below).

    rsx! {
        div { class: "relative",
            button {
                class: if active() { "btn btn-sm btn-ghost relative select-none" } else { "btn btn-sm btn-ghost relative select-none text-base-content/40" },
                "aria-label": "Sleep timer",
                "aria-pressed": "{active()}",
                onpointerdown: on_pointer_down,
                onpointerup: on_pointer_up,
                onpointerleave: move |_| {
                    let next = *press_gen.peek() + 1;
                    press_gen.set(next);
                },
                onpointercancel: move |_| {
                    let next = *press_gen.peek() + 1;
                    press_gen.set(next);
                },
                // Keyboard activation (the pointer path doesn't fire for keys, and
                // there's no `onclick` to double-fire). Enter/Space toggle the timer.
                onkeydown: move |e| {
                    let key = e.key().to_string();
                    if key == "Enter" || key == " " {
                        e.prevent_default();
                        player.peek().toggle_sleep();
                    }
                },
                // Suppress the native long-press context menu on touch devices.
                oncontextmenu: move |e| e.prevent_default(),
                Moon { class: "w-5 h-5" }
                if let Some(m) = minutes() {
                    span { class: "badge badge-primary badge-sm absolute -top-1 -right-1", "{m}" }
                }
            }

            if open() {
                // Outside-click backdrop (closes the menu); the panel sits above it.
                div {
                    class: "fixed inset-0 z-[60]",
                    onclick: move |_| open.set(false),
                }
                ul {
                    class: "menu absolute bottom-full right-0 mb-2 z-[70] bg-base-200 rounded-box p-2 shadow w-44",
                    li {
                        class: "menu-title",
                        if let Some(m) = minutes() {
                            "Sleep in {m} min"
                        } else {
                            "Sleep timer off"
                        }
                    }
                    li {
                        button {
                            onclick: move |_| player.peek().adjust_sleep(inc()),
                            "+{inc()} min"
                        }
                    }
                    li {
                        button {
                            onclick: move |_| player.peek().adjust_sleep(-inc()),
                            "−{inc()} min"
                        }
                    }
                }
            }
        }
    }
}
