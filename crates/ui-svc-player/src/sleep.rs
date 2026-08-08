//! Sleep-timer state + orchestration, owned by the
//! [`PlayerController`](super::PlayerController).
//!
//! [`SleepState`] is a small value type — the controller holds it in a
//! `Signal<SleepState>`, ticks it down with playback, and pauses on expiry. The UI
//! only reads it (active flag + remaining minutes for the badge); all mutation goes
//! through the controller's `toggle_sleep`/`disable_sleep`/`adjust_sleep`/auto-arm
//! methods, so the timer's rules live in one place. Those methods delegate to the free functions
//! below (`arm`/`disable`/`toggle`/`adjust`/`maybe_auto_arm`/`advance`), which
//! operate directly on the `Signal<SleepState>` (+ the once-per-session auto-arm
//! latch) so the orchestration lives next to the state it drives.

use std::cell::Cell;
use std::rc::Rc;

use dioxus::prelude::*;

use halogen_ui_logging::{debug, info};

/// How often the player polls/ticks, in milliseconds. Shared by the provider's poll
/// loop and the controller's sleep decrement so the countdown stays in step with the
/// tick cadence (DRY — one source of truth for the interval).
pub const TICK_INTERVAL_MS: u64 = 250;

/// Remaining sleep time. `None` = the timer is inactive.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct SleepState {
    remaining_secs: Option<f64>,
}

impl SleepState {
    /// An armed timer of `minutes` (clamped to ≥ 0). `0` or less yields the inactive
    /// state, so "arm with a non-positive duration" is the same as "disabled".
    pub fn from_minutes(minutes: i64) -> Self {
        Self {
            remaining_secs: (minutes > 0).then(|| (minutes * 60) as f64),
        }
    }

    /// Construct directly from a (positive) seconds remaining; non-positive = inactive.
    fn from_secs(secs: f64) -> Self {
        Self {
            remaining_secs: (secs > 0.0).then_some(secs),
        }
    }

    pub fn is_active(&self) -> bool {
        self.remaining_secs.is_some()
    }

    /// Whole minutes remaining, rounded **up** (so the badge reads `1` until the
    /// timer truly reaches zero). `None` when inactive.
    pub fn remaining_minutes(&self) -> Option<i64> {
        self.remaining_secs.map(|s| (s / 60.0).ceil() as i64)
    }

    /// Decrement by `dt` seconds. Returns `true` iff this tick crossed into expiry
    /// (was active, now inactive) — the controller uses that edge to pause once.
    pub(crate) fn tick_down(&mut self, dt: f64) -> bool {
        let Some(remaining) = self.remaining_secs else {
            return false;
        };
        let next = Self::from_secs(remaining - dt);
        let expired = !next.is_active();
        *self = next;
        expired
    }

    /// Add `delta_minutes` (may be negative) to the remaining time. Clamps at zero →
    /// inactive; adding to an inactive timer starts a fresh one at `delta_minutes`.
    pub(crate) fn adjusted(&self, delta_minutes: i64) -> Self {
        let current = self.remaining_secs.unwrap_or(0.0);
        Self::from_secs(current + (delta_minutes * 60) as f64)
    }
}

// ── Orchestration (operates on the controller's `Signal<SleepState>`) ────────
//
// The timer lives on the controller (not per-episode) so it spans auto-advance
// and queue transitions. It only ticks while actually playing (see the
// controller's `tick`), and pauses playback on expiry (`advance` reports the
// edge; the controller pauses). The controller's `stop` clears it.

/// Arm the sleep timer for `minutes` (≤ 0 disables).
pub(super) fn arm(mut sleep: Signal<SleepState>, minutes: i64) {
    debug!(minutes, "Arm sleep timer");
    sleep.set(SleepState::from_minutes(minutes));
}

/// Disable the sleep timer.
pub(super) fn disable(mut sleep: Signal<SleepState>) {
    sleep.set(SleepState::default());
}

/// Toggle the timer: active → off; off → armed with `default_minutes`.
pub(super) fn toggle(sleep: Signal<SleepState>, default_minutes: i64) {
    if sleep.peek().is_active() {
        disable(sleep);
    } else {
        arm(sleep, default_minutes);
    }
}

/// Nudge the remaining time by `delta_minutes` (clamped at 0 → off; from off, a
/// positive delta starts a fresh timer).
pub(super) fn adjust(mut sleep: Signal<SleepState>, delta_minutes: i64) {
    let next = sleep.peek().adjusted(delta_minutes);
    sleep.set(next);
}

/// Auto-arm the timer once per session when `sleep_by_default` is set and nothing
/// is armed yet. Called when playback is requested; the `auto_armed` latch keeps it
/// from re-arming on each episode, so the timer spans the whole session.
pub(super) fn maybe_auto_arm(
    sleep: Signal<SleepState>,
    auto_armed: &Rc<Cell<bool>>,
    sleep_by_default: bool,
    default_minutes: i64,
) {
    if auto_armed.get() || sleep.peek().is_active() {
        return;
    }
    if sleep_by_default {
        arm(sleep, default_minutes);
        auto_armed.set(true);
    }
}

/// Advance the timer by `dt` seconds. Returns `true` iff this tick crossed into
/// expiry — the controller pauses once on that edge. Called from the controller's
/// `tick` only while actually playing.
pub(super) fn advance(mut sleep: Signal<SleepState>, dt: f64) -> bool {
    let mut state = *sleep.peek();
    if !state.is_active() {
        return false;
    }
    // The displayed value (active flag + whole minutes for the badge) only changes
    // once a minute, but the countdown ticks every `TICK_INTERVAL_MS`. Snapshot the
    // display before ticking so we only *notify* `sleep_state` subscribers when it
    // actually changes — mirrors the value-gating in
    // `controller.rs::update_now_playing`, which avoids ~60× the renders.
    let displayed = (state.is_active(), state.remaining_minutes());
    let expired = state.tick_down(dt);
    if expired {
        info!("Sleep timer expired — pausing playback");
    }
    if (state.is_active(), state.remaining_minutes()) != displayed {
        // Minute rolled over (or the timer expired): publish so the badge re-renders.
        sleep.set(state);
    } else {
        // Sub-minute tick: persist the fine-grained seconds for the next tick
        // *without* notifying — a `set` here re-renders every badge subscriber for a
        // display that hasn't changed. The signal value and the badge are
        // intentionally a minute out of step until the next boundary's `set`
        // re-syncs them.
        #[allow(deprecated)]
        {
            *sleep.write_silent() = state;
        }
    }
    expired
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_minutes_clamps_non_positive_to_inactive() {
        assert!(!SleepState::from_minutes(0).is_active());
        assert!(!SleepState::from_minutes(-5).is_active());
        assert!(SleepState::from_minutes(30).is_active());
    }

    #[test]
    fn remaining_minutes_rounds_up() {
        // 30s left still shows "1" minute.
        let s = SleepState::from_secs(30.0);
        assert_eq!(s.remaining_minutes(), Some(1));
        // Exactly 2 minutes.
        assert_eq!(SleepState::from_minutes(2).remaining_minutes(), Some(2));
        // Inactive.
        assert_eq!(SleepState::default().remaining_minutes(), None);
    }

    #[test]
    fn tick_down_reports_expiry_edge_once() {
        let mut s = SleepState::from_secs(0.2);
        assert!(s.tick_down(0.25), "crossing zero reports expiry");
        assert!(!s.is_active());
        // Already inactive — no further expiry edge.
        assert!(!s.tick_down(0.25));
    }

    #[test]
    fn adjusted_clamps_and_starts_from_inactive() {
        // +5 from inactive starts a fresh 5-minute timer.
        assert_eq!(
            SleepState::default().adjusted(5).remaining_minutes(),
            Some(5)
        );
        // −X past zero disables.
        assert!(!SleepState::from_minutes(5).adjusted(-10).is_active());
    }
}
