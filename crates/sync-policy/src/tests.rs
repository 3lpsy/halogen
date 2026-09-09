use crate::*;

#[test]
fn throttle_and_auth_never_exhaust_the_retry_budget() {
    assert_eq!(decide_push(401, u32::MAX), PushDecision::AuthPaused);
    for status in [408, 429] {
        assert_eq!(
            decide_push(status, u32::MAX),
            PushDecision::StopAndRetryLater
        );
    }
    assert_eq!(decide_push(500, 8), PushDecision::CountAttemptAndStop);
    assert_eq!(decide_push(500, 9), PushDecision::Quarantine);
    assert_eq!(decide_push(400, 0), PushDecision::Quarantine);
}

#[test]
fn a_quiet_push_does_not_mask_a_failed_pull() {
    assert_eq!(
        cycle_status(Reach::Ok, Some(Reach::Offline)),
        CycleStatus {
            online: false,
            auth_paused: false
        }
    );
    assert_eq!(
        cycle_status(Reach::Ok, Some(Reach::AuthRejected)),
        CycleStatus {
            online: false,
            auth_paused: true
        }
    );
}
