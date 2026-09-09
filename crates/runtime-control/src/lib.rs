//! RestartHandle flags and wakes graceful shutdown. After Axum drains requests, main re-execs with the same argv/env so
//! Config::resolve reloads overrides and reconstructs startup-bound services.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio::sync::Notify;

/// Shared handle the HTTP layer uses to request a restart and `main` uses to
/// wait for one. Cheap to clone (two `Arc`s).
#[derive(Clone, Debug)]
pub struct RestartHandle {
    requested: Arc<AtomicBool>,
    notify: Arc<Notify>,
}

impl RestartHandle {
    pub fn new() -> Self {
        Self {
            requested: Arc::new(AtomicBool::new(false)),
            notify: Arc::new(Notify::new()),
        }
    }

    /// Flag a restart and wake the shutdown waiter. Idempotent.
    pub fn request(&self) {
        self.requested.store(true, Ordering::SeqCst);
        self.notify.notify_waiters();
    }

    /// Whether a restart was requested (checked by `main` after `serve` returns).
    pub fn is_requested(&self) -> bool {
        self.requested.load(Ordering::SeqCst)
    }

    /// Resolves when [`request`](Self::request) is called. Used as the
    /// graceful-shutdown trigger. The waiter registers when first polled (at
    /// serve start), so a `request` from a later API call is never missed.
    pub async fn wait(&self) {
        self.notify.notified().await;
    }
}

impl Default for RestartHandle {
    fn default() -> Self {
        Self::new()
    }
}
