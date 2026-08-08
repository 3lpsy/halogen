//! Re-enter the Dioxus runtime from browser-invoked callbacks.
//!
//! Raw `wasm_bindgen` closures (the Media Session action handlers, window /
//! document listeners) are invoked by the browser with NO Dioxus runtime on
//! the thread-local stack, so anything inside them that reaches
//! `dioxus::prelude::spawn` panics ("Must be called from inside a Dioxus
//! runtime") — and a wasm panic is unrecoverable, freezing the whole app.
//! [`ScopeBound`] captures the current `Rc<Runtime>` + `ScopeId` at
//! registration time (inside a component scope) and re-enters both around
//! each later invocation via `Runtime::in_scope` — the same pattern
//! dioxus-desktop's `use_wry_event_handler` uses for wry event-loop
//! callbacks. A bare `RuntimeGuard` would not be enough: `spawn` resolves the
//! *current scope*, which only `in_scope` establishes.
//!
//! The re-entry is SYNCHRONOUS: the callback body still runs on the caller's
//! stack. That is load-bearing for the web Media Session handlers, which must
//! stay on the browser's user-activation call stack so the autoplay
//! `PlayerBackend::prime` gesture latch keeps working (see `web.rs`) — this
//! is why those handlers are wrapped rather than routed through an async
//! channel pump like the webview variant. Tasks spawned inside the body are
//! owned by the captured scope and cancelled when it unmounts — the correct
//! lifetime for `PlayerProvider`, which remounts per user switch.

use std::rc::Rc;

use dioxus::core::{Runtime, ScopeId};

/// A captured `Runtime` + `ScopeId` that wraps callbacks so they always
/// execute inside that runtime and scope. Cheap to clone (an `Rc` + a `Copy`
/// id).
#[derive(Clone)]
pub struct ScopeBound {
    runtime: Rc<Runtime>,
    scope: ScopeId,
}

impl ScopeBound {
    /// Capture the current runtime + scope. Must be called from inside a
    /// component scope (component body / `use_hook` initializer); panics
    /// otherwise — a registration-time programming error that fails loudly at
    /// mount, not a runtime hazard in a browser callback.
    pub fn capture() -> Self {
        Self {
            runtime: Runtime::current(),
            scope: dioxus::core::current_scope_id(),
        }
    }

    /// Run `f` with the captured runtime + scope pushed onto the thread-local
    /// stacks (popped on return). Synchronous; nesting under an already-active
    /// runtime is fine — both are stacks.
    pub fn run<O>(&self, f: impl FnOnce() -> O) -> O {
        self.runtime.in_scope(self.scope, f)
    }

    /// Wrap an `FnMut()` callback so every invocation runs via [`Self::run`].
    pub fn bind(&self, mut f: impl FnMut() + 'static) -> impl FnMut() + 'static {
        let this = self.clone();
        // `&mut F where F: FnMut()` implements `FnOnce()`, so `run` can take
        // it by reference without consuming `f`.
        move || this.run(&mut f)
    }
}
