//! Run a JS-interop script that registers timers/observers/listeners and streams
//! values back to Rust, with **guaranteed teardown on unmount**.
//!
//! Why this exists: `document::eval` runs the script inside an async IIFE and only
//! fires its internal `dioxus.close()` when that body *returns*. A script that
//! parks forever (`setInterval` / `addEventListener` / `IntersectionObserver`)
//! never returns, so the browser resources it registers **leak on unmount** —
//! dropping the Rust `Eval` sends no teardown signal (verified against
//! dioxus-web 0.7.9: `Eval` is `Copy`, has no `Drop`, and the JS half is owned by
//! the browser event loop). Each list navigation would otherwise leave a live
//! interval + observer + window/document listeners behind.
//!
//! The fix: the script registers everything against an `AbortController` (exposed
//! to the body as `signal`) and then `await dioxus.recv()`. On unmount `use_drop`
//! sends a sentinel, the parked `await` resolves, the script aborts (removing every
//! listener/timer/observer at once) and returns — which triggers the clean
//! `dioxus.close()`. The `Eval` is created in the post-mount task (the script runs
//! synchronously, so it can't be made at render time — the element isn't in the DOM
//! yet) and stashed in a shared cell; its backing box is owned by the JS channel,
//! not the scope, so it stays valid for `use_drop` to send to even after the task
//! is cancelled (tasks drop before hooks).

use std::cell::RefCell;
use std::rc::Rc;

use dioxus::document::Eval;
use dioxus::prelude::*;
use serde::de::DeserializeOwned;

/// Mount a self-cleaning JS-interop stream for the lifetime of the calling
/// component. Call unconditionally (it's a hook).
///
/// `build_body` returns the JS that registers the listeners. It runs with two
/// variables in scope: `signal` (an `AbortSignal` — pass it to `addEventListener`
/// as `{ signal }`, and wire `setInterval`/`IntersectionObserver` teardown to
/// `signal.addEventListener('abort', …)`) and `dioxus` (for `dioxus.send`). Do
/// NOT add a trailing `return`/`await dioxus.recv()` — the harness appends the
/// park-and-abort. `on_msg` runs for each value the script sends back.
pub fn use_dom_stream<T: DeserializeOwned + 'static>(
    build_body: impl FnOnce() -> String + 'static,
    on_msg: impl FnMut(T) + 'static,
) {
    // Stash for the `Eval` handle, so `use_drop` can send the teardown sentinel.
    // `document::eval` runs its script *synchronously*, so it must be created
    // post-mount (inside the task below, like the old `use_future`) — at render
    // time the target element isn't in the DOM yet and `getElementById` would miss
    // it. The `Eval`'s backing box is owned by the JS channel (not this scope), so
    // it stays valid after the task is cancelled, right up until we close it.
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
