//! Wasm-only window/document listeners removed on unmount via `use_drop`. Explicit teardown prevents duplicate handlers
//! when `PlayerProvider` remounts for another user; native webviews do not link `web_sys`.

use std::rc::Rc;

use dioxus::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

use halogen_webui_player::ScopeBound;

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
