//! [`Tracked<T>`] — a working-state cell that flags itself dirty on any mutable
//! access, so the worker can publish only the domains that actually changed since
//! the last publish (selective per-domain re-render).
//!
//! The dirty bit flips on mutable *access* (`DerefMut`), **not** on value change.
//! That's deliberate: it preserves the "publish even when value-equal"
//! notification the episode list relies on — `cache_episodes` re-inserting an
//! already-pooled (value-equal) page must still re-render effect A, which uses the
//! publish as a store-changed signal (see `worker::SyncService::publish`).

use std::ops::{Deref, DerefMut};

/// One worker-owned working copy plus a dirty flag. Reads (`Deref`) never dirty
/// it; any `&mut` access (`DerefMut`) does. `publish` consults [`Tracked::take_dirty`]
/// to decide whether to push the matching signal.
pub(crate) struct Tracked<T> {
    val: T,
    dirty: bool,
}

impl<T> Tracked<T> {
    /// Wrap a value, starting **dirty** so the worker's first `publish` always
    /// emits the hydrated initial state into the signal.
    pub(crate) fn new(val: T) -> Self {
        Self { val, dirty: true }
    }

    /// Return whether the cell was touched since the last call, clearing the flag.
    pub(crate) fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }
}

impl<T> Deref for Tracked<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.val
    }
}

impl<T> DerefMut for Tracked<T> {
    fn deref_mut(&mut self) -> &mut T {
        self.dirty = true;
        &mut self.val
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_dirty_then_clears() {
        let mut t = Tracked::new(0u32);
        assert!(t.take_dirty(), "new() starts dirty for the initial publish");
        assert!(!t.take_dirty(), "stays clean with no access");
    }

    #[test]
    fn reads_dont_dirty_writes_do() {
        let mut t = Tracked::new(5u32);
        let _ = t.take_dirty(); // clear the initial flag
        assert_eq!(*t, 5); // Deref read
        assert!(!t.take_dirty(), "a read must not dirty the cell");
        *t += 1; // DerefMut write
        assert!(t.take_dirty(), "a write dirties the cell");
        assert_eq!(*t, 6);
    }
}
