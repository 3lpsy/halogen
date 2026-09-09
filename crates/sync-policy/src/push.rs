/// How a failed outbox replay should be treated (UI.md taxonomy).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureClass {
    /// Offline/timeout/throttle/auth: retry forever, never counted.
    Transient,
    /// 5xx / decode: retry against a small budget, then dead-letter.
    Countable,
    /// Validation-shaped 4xx: dead-letter immediately.
    Permanent,
}

/// Classify a non-2xx replay response status. Transport errors (no response
/// at all) are always [`FailureClass::Transient`].
pub fn classify_status(status: u16) -> FailureClass {
    match status {
        401 | 408 | 429 => FailureClass::Transient,
        400..=499 => FailureClass::Permanent,
        _ => FailureClass::Countable,
    }
}

pub const ATTEMPT_BUDGET: u32 = 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PushDecision {
    StopAndRetryLater,
    AuthPaused,
    CountAttemptAndStop,
    Quarantine,
}

pub fn decide_push(status: u16, attempts: u32) -> PushDecision {
    match classify_status(status) {
        FailureClass::Transient if status == 401 => PushDecision::AuthPaused,
        FailureClass::Transient => PushDecision::StopAndRetryLater,
        FailureClass::Permanent => PushDecision::Quarantine,
        FailureClass::Countable if attempts.saturating_add(1) >= ATTEMPT_BUDGET => {
            PushDecision::Quarantine
        }
        FailureClass::Countable => PushDecision::CountAttemptAndStop,
    }
}

pub fn drain_skips(failures: u32) -> u32 {
    (1u32 << failures.min(4)) - 1
}

pub fn status_of_error(error: &halogen_apiclient::ApiError) -> Option<u16> {
    use halogen_apiclient::ApiError;
    match error {
        ApiError::Server { status, .. } => Some(*status),
        ApiError::Validation(errors) => {
            let codes: Vec<&str> = errors
                .errors
                .values()
                .flatten()
                .map(|error| error.code.as_str())
                .collect();
            Some(if codes.contains(&"unauthenticated") {
                401
            } else if codes.contains(&"panic") {
                500
            } else if codes.contains(&"unimplemented") {
                501
            } else if codes.contains(&"unauthorized") {
                403
            } else if codes.contains(&"exists") {
                404
            } else if codes.contains(&"conflict") || codes.contains(&"unique") {
                409
            } else {
                400
            })
        }
        ApiError::Decode(_) | ApiError::Empty => Some(500),
        ApiError::Transport(_) => None,
    }
}

pub fn is_permanent_failure(error: &halogen_apiclient::ApiError) -> bool {
    status_of_error(error).is_some_and(|status| classify_status(status) == FailureClass::Permanent)
}

pub fn is_countable_failure(error: &halogen_apiclient::ApiError) -> bool {
    status_of_error(error).is_some_and(|status| classify_status(status) == FailureClass::Countable)
}
