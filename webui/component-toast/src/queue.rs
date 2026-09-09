use crate::classify::{ToastDecision, ToastPolicy, classify};

use dioxus::prelude::*;
use halogen_apiclient::ApiError;

/// Default auto-dismiss delay for transient toasts (ms).
pub const DEFAULT_TIMEOUT_MS: u32 = 5_000;
/// Errors linger a little longer so they're not missed.
pub const ERROR_TIMEOUT_MS: u32 = 8_000;

/// Severity of a toast — maps to a daisyUI `alert-*` modifier.
///
/// `Info`/`Success`/`Warning` are part of the reusable public API for callers
/// even though the error funnel itself only raises `Error` today.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[allow(dead_code)]
pub enum ToastLevel {
    Info,
    Success,
    Warning,
    Error,
}

impl ToastLevel {
    /// The daisyUI alert modifier class for this level.
    pub fn alert_class(self) -> &'static str {
        match self {
            ToastLevel::Info => "alert-info",
            ToastLevel::Success => "alert-success",
            ToastLevel::Warning => "alert-warning",
            ToastLevel::Error => "alert-error",
        }
    }

    /// The default auto-dismiss for this level.
    pub fn default_timeout_ms(self) -> u32 {
        match self {
            ToastLevel::Error => ERROR_TIMEOUT_MS,
            _ => DEFAULT_TIMEOUT_MS,
        }
    }
}

/// A single toast notification.
#[derive(Debug, Clone, PartialEq)]
pub struct Toast {
    pub id: u64,
    pub level: ToastLevel,
    pub message: String,
    /// Auto-dismiss delay in ms; `None` keeps it until dismissed.
    pub timeout_ms: Option<u32>,
}

/// The reactive toast queue. Provided as a `Signal<ToastQueue>` at the app root.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ToastQueue {
    pub toasts: Vec<Toast>,
    next_id: u64,
}

impl ToastQueue {
    /// Push a toast, returning its id. Deduplicates: an existing toast with the same level+message is dropped first, so
    /// an identical message re-raised (e.g. the worker retrying a failing op every 60s, or repeated 500s) refreshes the
    /// toast in place instead of stacking duplicates.
    pub fn push(
        &mut self,
        level: ToastLevel,
        message: impl Into<String>,
        timeout_ms: Option<u32>,
    ) -> u64 {
        let message = message.into();
        self.toasts
            .retain(|t| !(t.level == level && t.message == message));
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        self.toasts.push(Toast {
            id,
            level,
            message,
            timeout_ms,
        });
        id
    }

    /// Remove a toast by id (user dismiss or auto-dismiss timer).
    pub fn dismiss(&mut self, id: u64) {
        self.toasts.retain(|t| t.id != id);
    }
}

/// A `Copy` emitter over the global toast signal. Pass it into event handlers,
/// `spawn`ed futures, or the sync worker to raise toasts from anywhere.
#[derive(Clone, Copy)]
pub struct ToastHandle {
    store: Signal<ToastQueue>,
}

impl ToastHandle {
    pub fn new(store: Signal<ToastQueue>) -> Self {
        Self { store }
    }

    /// Raise a toast at the given level.
    pub fn show(&self, level: ToastLevel, message: impl Into<String>, timeout_ms: Option<u32>) {
        let mut store = self.store;
        store.write().push(level, message, timeout_ms);
    }

    // Convenience emitters — reusable public API for callers (success/info/warn
    // toasts), not all exercised by the error funnel yet.
    #[allow(dead_code)]
    pub fn info(&self, message: impl Into<String>) {
        self.show(ToastLevel::Info, message, Some(DEFAULT_TIMEOUT_MS));
    }
    #[allow(dead_code)]
    pub fn success(&self, message: impl Into<String>) {
        self.show(ToastLevel::Success, message, Some(DEFAULT_TIMEOUT_MS));
    }
    #[allow(dead_code)]
    pub fn warn(&self, message: impl Into<String>) {
        self.show(ToastLevel::Warning, message, Some(DEFAULT_TIMEOUT_MS));
    }
    pub fn error(&self, message: impl Into<String>) {
        self.show(ToastLevel::Error, message, Some(ERROR_TIMEOUT_MS));
    }

    /// Apply a classified [`ToastDecision`] from the funnel: a `Toast(..)`
    /// decision is shown; `Offline`/`SignOut`/`Silent` are handled by the caller
    /// (they don't toast here). Returns the decision so callers can act on it.
    pub fn apply(&self, decision: ToastDecision) -> ToastDecision {
        if let ToastDecision::Toast(level, ref message) = decision {
            self.show(level, message.clone(), Some(level.default_timeout_ms()));
        }
        decision
    }
}

/// Funnel for foreground (synchronous) API calls. Classify the error per `policy`, raise a toast for the cases that
/// warrant one, and return the original `Result` untouched so the caller can still match specific variants (e.g.
/// `Validation`/`Transport`) for inline form handling.
pub trait ApiResultExt<T> {
    fn report(self, toast: &ToastHandle, policy: ToastPolicy) -> Result<T, ApiError>;
}

impl<T> ApiResultExt<T> for Result<T, ApiError> {
    fn report(self, toast: &ToastHandle, policy: ToastPolicy) -> Result<T, ApiError> {
        if let Err(ref e) = self {
            toast.apply(classify(e, policy));
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── ToastQueue::push dedup + id sequencing ──────────────────────────────

    #[test]
    fn push_dedups_same_level_and_message() {
        let mut store = ToastQueue::default();
        store.push(ToastLevel::Error, "boom", None);
        store.push(ToastLevel::Error, "boom", None);
        assert_eq!(store.toasts.len(), 1, "identical toast refreshes in place");
        assert_eq!(store.toasts[0].message, "boom");
    }

    #[test]
    fn push_stacks_different_messages_and_levels() {
        let mut store = ToastQueue::default();
        store.push(ToastLevel::Error, "a", None);
        store.push(ToastLevel::Error, "b", None);
        // Same message, different level → not a dup.
        store.push(ToastLevel::Info, "a", None);
        assert_eq!(store.toasts.len(), 3);
    }

    #[test]
    fn push_returns_incrementing_ids() {
        let mut store = ToastQueue::default();
        let id0 = store.push(ToastLevel::Info, "a", None);
        let id1 = store.push(ToastLevel::Info, "b", None);
        let id2 = store.push(ToastLevel::Info, "c", None);
        assert_eq!(id0, 0);
        assert_eq!(id1, id0.wrapping_add(1));
        assert_eq!(id2, id1.wrapping_add(1));
        // Re-raising a dup still consumes a fresh id (dedup removes the old row,
        // pushes a new one with the next id).
        let dup = store.push(ToastLevel::Info, "a", None);
        assert_eq!(dup, 3);
    }

    // ── dismiss ─────────────────────────────────────────────────────────────

    #[test]
    fn dismiss_removes_only_the_targeted_id() {
        let mut store = ToastQueue::default();
        let a = store.push(ToastLevel::Info, "a", None);
        let b = store.push(ToastLevel::Info, "b", None);
        let c = store.push(ToastLevel::Info, "c", None);
        store.dismiss(b);
        let ids: Vec<u64> = store.toasts.iter().map(|t| t.id).collect();
        assert_eq!(ids, vec![a, c]);
        // Dismissing an unknown id is a no-op.
        store.dismiss(999);
        assert_eq!(store.toasts.len(), 2);
    }

    // ── ToastLevel helpers ──────────────────────────────────────────────────

    #[test]
    fn alert_class_per_variant() {
        assert_eq!(ToastLevel::Info.alert_class(), "alert-info");
        assert_eq!(ToastLevel::Success.alert_class(), "alert-success");
        assert_eq!(ToastLevel::Warning.alert_class(), "alert-warning");
        assert_eq!(ToastLevel::Error.alert_class(), "alert-error");
    }

    #[test]
    fn default_timeout_ms_per_variant() {
        assert_eq!(ToastLevel::Error.default_timeout_ms(), ERROR_TIMEOUT_MS);
        assert_eq!(ToastLevel::Info.default_timeout_ms(), DEFAULT_TIMEOUT_MS);
        assert_eq!(ToastLevel::Success.default_timeout_ms(), DEFAULT_TIMEOUT_MS);
        assert_eq!(ToastLevel::Warning.default_timeout_ms(), DEFAULT_TIMEOUT_MS);
    }
}
