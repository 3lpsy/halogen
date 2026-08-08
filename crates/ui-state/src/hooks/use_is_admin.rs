use dioxus::prelude::*;

use crate::hooks::use_config;

/// Whether the signed-in user is a server admin, as a `PartialEq`-gated memo
/// slice.
///
/// Sourced from `ClientConfig.is_admin`, populated after login by fetching the
/// current user. UI-gating only (the Admin nav item / `/admin` routes); the
/// server is the real authority — never a security boundary.
///
/// Returns a `Memo` (not a bare `bool`) so consumers don't clone the whole
/// `ClientConfig` each render and only re-render when `is_admin` itself flips —
/// the same slice discipline as [`crate::hooks::use_sync_status`].
pub fn use_is_admin() -> Memo<bool> {
    let config = use_config();
    use_memo(move || config.read().is_admin)
}
