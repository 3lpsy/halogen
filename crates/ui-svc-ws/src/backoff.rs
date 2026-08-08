//! Reconnect backoff schedule for the connectivity [`driver`](super::driver).

use halogen_ui_logging::debug;
use halogen_ui_platform::time::sleep_ms;

/// Reconnect backoff: 1s ×10, then 2s ×5, then exponential 4, 8, 16, 32, steady 60s.
/// Keeps reconnects tight right after a blip (likely transient) before backing off.
pub(super) struct Backoff {
    /// Number of waits since the last reset; drives the schedule.
    count: u32,
}

/// Fast phase: 1s waits before easing off.
const BACKOFF_FAST_TRIES: u32 = 10;
/// Medium phase: 2s waits after the fast phase.
const BACKOFF_MED_TRIES: u32 = 5;

impl Backoff {
    pub(super) fn new() -> Self {
        Self { count: 0 }
    }
    pub(super) fn reset(&mut self) {
        self.count = 0;
    }
    fn delay_secs(&self) -> u64 {
        if self.count < BACKOFF_FAST_TRIES {
            1
        } else if self.count < BACKOFF_FAST_TRIES + BACKOFF_MED_TRIES {
            2
        } else {
            // Exponential from 4s, capped at 60s. Clamp the shift so it can't overflow.
            let step = (self.count - (BACKOFF_FAST_TRIES + BACKOFF_MED_TRIES)).min(6);
            (4u64 << step).min(60)
        }
    }
    pub(super) async fn wait(&mut self) {
        let secs = self.delay_secs();
        debug!(secs, "WS reconnect backoff");
        sleep_ms((secs * 1000) as u32).await;
        self.count += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_schedule_is_1x10_then_2x5_then_exponential() {
        let mut b = Backoff::new();
        let mut seq = Vec::new();
        for _ in 0..22 {
            seq.push(b.delay_secs());
            b.count += 1; // advance the schedule without actually sleeping
        }
        #[rustfmt::skip]
        let expected = vec![
            1, 1, 1, 1, 1, 1, 1, 1, 1, 1, // fast phase ×10
            2, 2, 2, 2, 2,                // medium phase ×5
            4, 8, 16, 32, 60, 60, 60,     // exponential, capped at 60
        ];
        assert_eq!(seq, expected);
    }

    #[test]
    fn backoff_reset_returns_to_one() {
        let mut b = Backoff::new();
        b.count = 30;
        b.reset();
        assert_eq!(b.delay_secs(), 1);
    }
}
