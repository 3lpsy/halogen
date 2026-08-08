//! Admin-only config-overrides editor (`/settings/config/overrides`).
//!
//! Reached from the View Config header's pencil. Lets an admin add, edit, and
//! remove entries in the server's writable override allowlist
//! ([`ConfigOverridesData`]). The flow is "replace-all": on Save we POST the full
//! working set, so a param removed here is deleted server-side. Saving and the
//! "Clear all" action each confirm first; neither restarts — the View Config
//! header's restart button applies the changes.
//!
//! Online + admin only: the mutating actions are disabled offline and a deep-link
//! by a non-admin is refused. Each overridable parameter renders with the input
//! that fits its type — booleans as a select, the playback percentage bounded
//! 0–100, `no_sync_before` as a date, the rest numeric/text.

mod page;
mod params;

pub use page::ConfigOverridesEdit;
