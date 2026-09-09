use dioxus::prelude::*;

use halogen_webui_hook_context::use_config;

/// Memoize the signed-in user's admin flag for navigation/UI gating. The server remains the authorization boundary; the
/// memo avoids rerenders on unrelated config changes.
pub fn use_is_admin() -> Memo<bool> {
    let config = use_config();
    use_memo(move || config.read().is_admin)
}
