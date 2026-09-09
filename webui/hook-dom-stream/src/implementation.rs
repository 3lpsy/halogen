//! Stream JS events with unmount cleanup: register resources on `signal`, then park on `dioxus.recv()`. `use_drop`
//! sends a sentinel so abort runs and the script returns, closing the channel. Create the eval post-mount and retain
//! its channel-owned handle beyond task cancellation; dropping `Eval` alone does not clean up browser resources.

use std::cell::RefCell;
use std::rc::Rc;

use dioxus::document::Eval;
use dioxus::prelude::*;
use serde::de::DeserializeOwned;

/// Call unconditionally. `build_body` receives `signal` and `dioxus`; register listeners with `{ signal }` and clear
/// timers/observers on abort. The harness appends the receive/abort sequence, so add no trailing return or receive.
/// `on_msg` handles each emitted value.
pub fn use_dom_stream<T: DeserializeOwned + 'static>(
    build_body: impl FnOnce() -> String + 'static,
    on_msg: impl FnMut(T) + 'static,
) {
    // Create the eval after mount so target DOM nodes exist. Retain its channel-owned handle for `use_drop`, which
    // sends cleanup after the task has been cancelled.
    let slot: Rc<RefCell<Option<Eval>>> = use_hook(|| Rc::new(RefCell::new(None)));

    let task_slot = slot.clone();
    use_hook(move || {
        // Build the script once at mount. An empty body means "nothing to register"
        // (e.g. a standalone list mounted without a scroll store) — skip the task +
        // eval entirely so the no-op case costs nothing. Building the string here at
        // render is fine: it's pure; only the eval must be deferred (next comment).
        let body = build_body();
        if body.is_empty() {
            return;
        }
        // The script EXECUTES synchronously when `document::eval` runs, so create it
        // in the task (first polled post-commit, like the old `use_future`) — the
        // target element is in the DOM by then.
        spawn(async move {
            let script = format!(
                "const __ac = new AbortController();\n\
                 const signal = __ac.signal;\n\
                 {body}\n\
                 await dioxus.recv();\n\
                 __ac.abort();"
            );
            let mut eval = document::eval(&script);
            *task_slot.borrow_mut() = Some(eval);
            let mut on_msg = on_msg;
            while let Ok(v) = eval.recv::<T>().await {
                on_msg(v);
            }
        });
    });

    // On unmount, wake the parked script so it aborts every registration and
    // returns (firing `dioxus.close()`). Any payload works — JS just needs a value.
    // (The drain task is already cancelled by now — tasks drop before hooks — but
    // the JS half is still parked on `dioxus.recv()` waiting for exactly this.)
    use_drop(move || {
        if let Some(eval) = *slot.borrow() {
            let _ = eval.send(true);
        }
    });
}
