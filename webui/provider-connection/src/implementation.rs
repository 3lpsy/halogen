use dioxus::prelude::*;

use halogen_webui_app_state::ConnectionState;

/// Worker-written connectivity/sync state lives separately from episode data so latency pongs and activity changes
/// update only subscribed status/offline controls.
#[component]
pub fn ConnectionStateProvider(children: Element) -> Element {
    let connection = use_signal(ConnectionState::default);
    use_context_provider(|| connection);
    rsx! { {children} }
}
