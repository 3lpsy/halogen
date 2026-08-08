//! A monotonic "latest wins" token for dropping stale in-flight async results.
//!
//! Several effects spawn an async task whose result must be discarded if a newer
//! run of the same effect started while it awaited (revalidating a list page,
//! debouncing a list-view-state save). They all want the same dance: bump a
//! generation counter synchronously before spawning, capture it, and re-check it
//! after the await. This packages that so the subtle `peek`/`set` ordering lives
//! in one place.

use dioxus::prelude::*;

/// A generation counter for "latest wins" cancellation. `Copy`, so it can be
/// captured by the spawned task.
#[derive(Clone, Copy)]
pub struct LatestWins(Signal<u64>);

/// Create a [`LatestWins`] token (generation `0`).
pub fn use_latest_wins() -> LatestWins {
    LatestWins(use_signal(|| 0u64))
}

impl LatestWins {
    /// Claim the next generation (synchronously, before spawning). Capture the
    /// returned value and pass it to [`is_current`](LatestWins::is_current) after
    /// the await to detect whether a newer claim superseded this one.
    pub fn claim(&self) -> u64 {
        let mut signal = self.0;
        let next = *signal.peek() + 1;
        signal.set(next);
        next
    }

    /// Whether `generation` is still the latest claim (no newer one started). When
    /// `false`, the caller should drop its stale result and return.
    pub fn is_current(&self, generation: u64) -> bool {
        *self.0.peek() == generation
    }
}
