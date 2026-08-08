use dioxus::prelude::*;

use halogen_ui_appstate::ConnectionState;

/// Provides the shared `Signal<ConnectionState>` (connectivity + sync-activity
/// status). The sync worker is the only writer; components read it via the
/// [`use_connection`](crate::hooks::use_connection) family of hooks.
///
/// A separate signal from `EpisodeState`, so the connectivity WebSocket's ~4×/sec
/// latency pongs (and Syncing/Saving churn) re-render only the navbar status dot +
/// offline-gated controls. Follows the same shape as `EpisodeStateProvider`.
#[component]
pub fn ConnectionStateProvider(children: Element) -> Element {
    let connection = use_signal(ConnectionState::default);
    use_context_provider(|| connection);
    rsx! { {children} }
}
