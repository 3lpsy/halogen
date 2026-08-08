//! Ephemeral, in-memory store type for the last Discover search.
//!
//! Discover is online-only and never persisted server-side, so results live only
//! here for the session. The `/discover` page is the sole writer; the
//! `/discover/:id` detail page reads a result by its synthetic id. Holding an
//! ordered `Vec` (not a map) preserves provider ranking and lets the list survive
//! navigating to a detail and back. A hard refresh / shared deep-link finds an
//! empty store and shows a "search again" fallback — by design.
//!
//! The shared `Signal<DiscoverState>` is installed by `DiscoverStateProvider` and
//! read via `use_discover_store` (both in halogen-ui-state). The type lives here
//! (with the other shared domain models) rather than in the provider.

use halogen_wire::DiscoverResultItem;

/// The last Discover search and its results.
#[derive(Clone, Default, PartialEq)]
pub struct DiscoverState {
    /// The query that produced `results` (echoed back into the search box).
    pub query: String,
    /// Results in provider/rank order.
    pub results: Vec<DiscoverResultItem>,
}

impl DiscoverState {
    /// Look a result up by its synthetic id (linear scan — result sets are small).
    pub fn get(&self, id: &str) -> Option<&DiscoverResultItem> {
        self.results.iter().find(|r| r.id == id)
    }
}
