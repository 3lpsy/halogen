//! Cross-tab/context mutual exclusion over the browser Web Locks API
//! (`navigator.locks`) — wasm only.
//!
//! Why this exists: every tab (and the sync Web Worker inside each) is a full,
//! independent instance of the app sharing one origin's IndexedDB. Two tabs on
//! the same account would double-drain the shared outbox and run dueling media
//! writers over one chunk keyspace. A Web Lock is scoped to the origin, works
//! in both window and worker contexts, and — crucially — is auto-released when
//! its holder's context dies, so a crashed or closed tab can never wedge the
//! others the way a persisted flag would.
//!
//! The API is reached dynamically via `Reflect` (no `web-sys` feature
//! plumbing, and it resolves in window *and* worker globals alike). Every
//! entry point degrades to [`TryLock::Unsupported`] on browsers without it
//! (Safari < 15.4) or when a request misbehaves — callers then proceed
//! unguarded, which is exactly the pre-lock status quo.

use std::cell::RefCell;
use std::rc::Rc;

use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};

/// A held Web Lock. Dropping it releases the lock (it resolves the pending
/// promise the lock-grant callback returned to the browser).
pub struct WebLockGuard {
    release: Option<js_sys::Function>,
}

impl Drop for WebLockGuard {
    fn drop(&mut self) {
        if let Some(release) = self.release.take() {
            let _ = release.call0(&JsValue::NULL);
        }
    }
}

/// Outcome of [`try_acquire`].
pub enum TryLock {
    /// The lock is ours; release by dropping the guard.
    Acquired(WebLockGuard),
    /// Another tab/worker holds it right now.
    Busy,
    /// No usable Web Locks API in this context — proceed unguarded.
    Unsupported,
}

/// `navigator.locks`, if this context has a usable Web Locks API.
fn lock_manager() -> Option<JsValue> {
    let global = js_sys::global();
    let navigator = js_sys::Reflect::get(&global, &JsValue::from_str("navigator")).ok()?;
    if navigator.is_undefined() || navigator.is_null() {
        return None;
    }
    let locks = js_sys::Reflect::get(&navigator, &JsValue::from_str("locks")).ok()?;
    (!locks.is_undefined() && !locks.is_null()).then_some(locks)
}

/// Acquire `name` exclusively, waiting for the current holder to release.
/// `None` = no usable API (or the request itself failed) — proceed unguarded.
pub async fn acquire(name: &str) -> Option<WebLockGuard> {
    match request(name, false).await {
        TryLock::Acquired(guard) => Some(guard),
        TryLock::Busy | TryLock::Unsupported => None,
    }
}

/// Acquire `name` exclusively only if it's free right now (`ifAvailable`) —
/// for long-held locks where waiting would mean silently duplicating work.
pub async fn try_acquire(name: &str) -> TryLock {
    request(name, true).await
}

async fn request(name: &str, if_available: bool) -> TryLock {
    let Some(locks) = lock_manager() else {
        return TryLock::Unsupported;
    };
    let Ok(request_fn) = js_sys::Reflect::get(&locks, &JsValue::from_str("request"))
        .and_then(|f| f.dyn_into::<js_sys::Function>().map_err(JsValue::from))
    else {
        return TryLock::Unsupported;
    };

    // The grant callback resolves this with the release function (`Some`) or
    // `None` when `ifAvailable` lost the race (the callback received `null`).
    let (granted_tx, granted_rx) = futures_channel::oneshot::channel::<Option<js_sys::Function>>();
    let granted_tx = Rc::new(RefCell::new(Some(granted_tx)));

    // Runs exactly once; returning a pending promise keeps the lock held until
    // we resolve it — that resolver is the guard's `release`.
    let cb_tx = granted_tx.clone();
    let callback: JsValue = Closure::once_into_js(move |lock: JsValue| -> js_sys::Promise {
        if lock.is_null() || lock.is_undefined() {
            if let Some(tx) = cb_tx.borrow_mut().take() {
                let _ = tx.send(None);
            }
            return js_sys::Promise::resolve(&JsValue::UNDEFINED);
        }
        let mut release: Option<js_sys::Function> = None;
        let hold = js_sys::Promise::new(&mut |resolve, _reject| {
            release = Some(resolve);
        });
        if let Some(tx) = cb_tx.borrow_mut().take() {
            let _ = tx.send(release);
        }
        hold
    });

    let options = js_sys::Object::new();
    if if_available {
        let _ = js_sys::Reflect::set(&options, &JsValue::from_str("ifAvailable"), &JsValue::TRUE);
    }
    let Ok(promise) = request_fn.call3(&locks, &JsValue::from_str(name), &options, &callback)
    else {
        return TryLock::Unsupported;
    };
    // If the request promise REJECTS before the callback fired (pathological:
    // bad name, API quirk), unblock the waiter so callers proceed unguarded
    // instead of hanging forever on `granted_rx`. Attached via `Reflect` like
    // the rest of this module (the typed `Promise::catch` wants a
    // `ScopedClosure`, which doesn't fit a fire-once detached handler).
    if let Ok(catch_fn) = js_sys::Reflect::get(&promise, &JsValue::from_str("catch"))
        .and_then(|f| f.dyn_into::<js_sys::Function>().map_err(JsValue::from))
    {
        let err_tx = granted_tx.clone();
        let on_err: JsValue = Closure::once_into_js(move |_e: JsValue| {
            if let Some(tx) = err_tx.borrow_mut().take() {
                let _ = tx.send(None);
            }
        });
        let _ = catch_fn.call1(&promise, &on_err);
    }

    match granted_rx.await {
        Ok(Some(release)) => TryLock::Acquired(WebLockGuard {
            release: Some(release),
        }),
        Ok(None) => TryLock::Busy,
        Err(_) => TryLock::Unsupported,
    }
}
