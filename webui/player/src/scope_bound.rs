//! Browser callbacks must synchronously re-enter captured Dioxus runtime and scope before spawning. A runtime guard
//! alone lacks scope; an async channel would lose autoplay user activation. [`ScopeBound`] preserves both and ties
//! spawned tasks to the provider lifetime.

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
