//! The error funnel: maps an [`ApiError`] to a [`ToastDecision`].
//!
//! This is the single source of truth for how each HTTP failure surfaces to the
//! user, shared by the foreground `report` path and the background sync worker.

use halogen_api::ApiError;
use halogen_wire::SerializableValidationErrors;

use super::ToastLevel;

/// Where an API error originated — changes how some errors surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastPolicy {
    /// A synchronous call tied to a form/screen that renders its own inline
    /// errors (login, server setup). Validation stays inline (no toast).
    Foreground,
    /// A call offloaded to the background sync worker — there is no form to fall
    /// back on, so validation must bubble up as a toast.
    Background,
}

/// What the funnel decides to do with an error.
#[derive(Debug, Clone, PartialEq)]
pub enum ToastDecision {
    /// Show this toast.
    Toast(ToastLevel, String),
    /// Connectivity failure — caller reflects Offline status, does not toast.
    Offline,
    /// Auth is dead (401) — caller signs the user out.
    SignOut,
    /// Do nothing (e.g. validation handled inline on a form).
    Silent,
}

/// Map an [`ApiError`] to a [`ToastDecision`] for the given policy.
pub fn classify(err: &ApiError, policy: ToastPolicy) -> ToastDecision {
    // Transport failure == offline; the navbar indicator carries this, no toast.
    // Via `ApiError::is_offline` so this funnel and the list pages' offline
    // handling can't disagree about what counts as unreachable.
    if err.is_offline() {
        return ToastDecision::Offline;
    }
    match err {
        // Token invalid/expired — sign out regardless of where it happened.
        ApiError::Server { status: 401, .. } => ToastDecision::SignOut,

        // Server-side faults: a generic, reassuring message (don't leak internals).
        ApiError::Server { status, .. } if (500..=599).contains(status) => ToastDecision::Toast(
            ToastLevel::Error,
            "Something went wrong on the server. Please try again.".to_string(),
        ),

        // Other non-2xx with a server-provided message.
        ApiError::Server { status, message } => {
            ToastDecision::Toast(ToastLevel::Error, friendly(*status, message))
        }

        // Handled above via `is_offline`.
        ApiError::Transport(_) => ToastDecision::Offline,

        // Validation: inline on forms, but bubbled as a toast off the form path.
        ApiError::Validation(errs) => match policy {
            ToastPolicy::Foreground => ToastDecision::Silent,
            ToastPolicy::Background => {
                ToastDecision::Toast(ToastLevel::Error, summarize_validation(errs))
            }
        },

        // Malformed / empty success body — unexpected, but not the user's fault.
        ApiError::Decode(_) | ApiError::Empty => ToastDecision::Toast(
            ToastLevel::Error,
            "Unexpected response from the server.".to_string(),
        ),
    }
}

/// Whether a failed outbox op should be dead-lettered (dropped) rather than
/// retried forever. A non-auth client error (4xx) or a validation rejection will
/// never succeed on replay, so retrying it every drain just re-toasts and keeps a
/// dead op at the head of the queue.
///
/// Excluded (kept queued, treated as transient): 401 drives sign-out (the token may
/// refresh); 408 (Request Timeout) and 429 (Too Many Requests) are explicitly
/// *retryable* — a rate-limited or timed-out action must NOT be silently dropped, it
/// should drain on the next pass. 5xx, transport, and decode/empty are transient too.
pub fn is_permanent_failure(err: &ApiError) -> bool {
    match err {
        // Retryable status codes: leave queued for the next drain.
        ApiError::Server { status, .. } if matches!(status, 401 | 408 | 429) => false,
        ApiError::Server { status, .. } => (400..=499).contains(status),
        ApiError::Validation(_) => true,
        ApiError::Transport(_) | ApiError::Decode(_) | ApiError::Empty => false,
    }
}

/// Whether a retryable failure counts against a bounded retry budget (the
/// outbox drain's head-op budget). These classes *can* be deterministic — a
/// server bug that 500s on this exact payload, or a `Decode`/`Empty` from
/// client/server version skew — in which case unbudgeted retry loops forever.
/// Excluded: `Transport` (offline is the normal local-first state; the queue
/// must survive it indefinitely), 401 (drives sign-out, token may refresh),
/// and 408/429 (explicitly throttle-and-retry).
pub fn is_countable_failure(err: &ApiError) -> bool {
    match err {
        ApiError::Server { status, .. } => *status >= 500,
        ApiError::Decode(_) | ApiError::Empty => true,
        ApiError::Transport(_) | ApiError::Validation(_) => false,
    }
}

/// Human-readable text for a non-5xx server error, preferring the server message.
fn friendly(status: u16, message: &str) -> String {
    let msg = message.trim();
    if msg.is_empty() {
        format!("Request failed (error {status}).")
    } else {
        msg.to_string()
    }
}

/// Condense field validation errors into one line for a toast (the first
/// available field message, else a generic fallback).
fn summarize_validation(errs: &SerializableValidationErrors) -> String {
    let first = errs
        .errors
        .values()
        .flatten()
        .find_map(|f| f.message.clone());
    match first {
        Some(msg) => format!("Couldn't save: {msg}"),
        None => "Couldn't save: some values were rejected.".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn server(status: u16, message: &str) -> ApiError {
        ApiError::Server {
            status,
            message: message.to_string(),
        }
    }

    #[test]
    fn unauthorized_signs_out_in_any_policy() {
        assert_eq!(
            classify(&server(401, "nope"), ToastPolicy::Foreground),
            ToastDecision::SignOut
        );
        assert_eq!(
            classify(&server(401, "nope"), ToastPolicy::Background),
            ToastDecision::SignOut
        );
    }

    #[test]
    fn server_5xx_is_a_generic_error_toast() {
        match classify(&server(500, "stacktrace leak"), ToastPolicy::Background) {
            ToastDecision::Toast(ToastLevel::Error, msg) => {
                assert!(!msg.contains("stacktrace"));
            }
            other => panic!("expected Toast(Error, _), got {other:?}"),
        }
    }

    #[test]
    fn validation_differs_by_policy() {
        let errs = SerializableValidationErrors {
            errors: HashMap::new(),
        };
        assert_eq!(
            classify(&ApiError::Validation(errs.clone()), ToastPolicy::Foreground),
            ToastDecision::Silent
        );
        assert!(matches!(
            classify(&ApiError::Validation(errs), ToastPolicy::Background),
            ToastDecision::Toast(ToastLevel::Error, _)
        ));
    }

    #[test]
    fn permanent_failure_excludes_retryable_statuses() {
        // Dead-letter genuine client rejections…
        assert!(is_permanent_failure(&server(400, "bad")));
        assert!(is_permanent_failure(&server(404, "gone")));
        assert!(is_permanent_failure(&server(422, "invalid")));
        // …but keep auth + retryable timeout/rate-limit queued for the next drain.
        assert!(!is_permanent_failure(&server(401, "expired")));
        assert!(!is_permanent_failure(&server(408, "request timeout")));
        assert!(!is_permanent_failure(&server(429, "slow down")));
        // Transport/5xx stay queued too.
        assert!(!is_permanent_failure(&server(503, "unavailable")));
        assert!(!is_permanent_failure(&ApiError::Empty));
    }

    #[test]
    fn countable_failure_is_5xx_decode_empty_only() {
        // Can be deterministic (server bug / version skew) → budgeted retry.
        assert!(is_countable_failure(&server(500, "boom")));
        assert!(is_countable_failure(&server(503, "unavailable")));
        assert!(is_countable_failure(&ApiError::Decode("x".into())));
        assert!(is_countable_failure(&ApiError::Empty));
        // Auth / throttling must retry forever, unbudgeted. (`Transport` — also
        // excluded — wraps a `reqwest::Error` and can't be built here; the
        // drain-level test in `ui-svc-sync::network` covers it end-to-end.)
        assert!(!is_countable_failure(&server(401, "expired")));
        assert!(!is_countable_failure(&server(408, "request timeout")));
        assert!(!is_countable_failure(&server(429, "slow down")));
    }

    #[test]
    fn decode_and_empty_are_unexpected_toasts() {
        assert!(matches!(
            classify(&ApiError::Decode("x".into()), ToastPolicy::Background),
            ToastDecision::Toast(ToastLevel::Error, _)
        ));
        assert!(matches!(
            classify(&ApiError::Empty, ToastPolicy::Foreground),
            ToastDecision::Toast(ToastLevel::Error, _)
        ));
    }
}
