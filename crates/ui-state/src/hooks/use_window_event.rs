//! RAII subscription to a global `window`/`document` event.
//!
//! Dioxus has no built-in hook for non-element (window/document) events, and
//! `document::eval` would leak the listener on unmount (see [`super::use_dom_stream`]
//! for the mechanism). This registers a `web_sys` listener at mount and removes it
//! on unmount via `use_drop`. Real teardown matters here: the provider that mounts
//! these (`PlayerProvider`) **remounts on a user switch**, so a `Closure::forget`
//! style registration would stack a duplicate handler per account.
//!
//! wasm-only: the other targets render in a webview too, but `web_sys` is only
//! linked on `wasm32`, and the data-loss window this addresses is the
//! mobile-browser PWA.

use std::rc::Rc;

use dioxus::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

use halogen_ui_svc_player::ScopeBound;

/// Subscribe `cb` to `event` on `target` (a `Window` or `Document` as an
/// `EventTarget`) for the lifetime of the calling component. Call unconditionally
/// (it's a hook). The listener is removed and its JS shim freed on unmount.
pub fn use_window_event(
    target: web_sys::EventTarget,
    event: &'static str,
    cb: impl FnMut() + 'static,
) {
    // Keep the `Closure` alive in a hook (dropping it frees the JS shim) and hold
    // the target + event so `use_drop` can detach with the same function ref.
    let handle = use_hook(move || {
        // The browser fires this listener with no Dioxus runtime on the
        // stack; re-enter the registering component's runtime + scope per
        // event so the callback can safely use spawn/signals/coroutines.
        let mut cb = ScopeBound::capture().bind(cb);
        let closure = Closure::wrap(Box::new(move |_: web_sys::Event| cb()) as Box<dyn FnMut(_)>);
        // A failed registration degrades (no listener) rather than panicking.
        let _ = target.add_event_listener_with_callback(event, closure.as_ref().unchecked_ref());
        Rc::new((target, closure))
    });
    use_drop({
        let handle = handle.clone();
        move || {
            let (target, closure) = &*handle;
            let _ =
                target.remove_event_listener_with_callback(event, closure.as_ref().unchecked_ref());
        }
    });
}
