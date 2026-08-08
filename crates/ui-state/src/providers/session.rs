use dioxus::prelude::*;

use halogen_ui_appstate::SessionState;

/// Provides the shared `Signal<SessionState>` (the worker-owned `auth_expired`
/// bit). The sync worker is the only writer; `WorkerProvider` watches it to clear
/// the stored token and sign out on a 401.
///
/// A separate signal from `EpisodeState`, so a 401 flips only the sign-out watcher, not
/// every `EpisodeState` subscriber. Follows the same shape as `EpisodeStateProvider`.
#[component]
pub fn SessionStateProvider(children: Element) -> Element {
    let session = use_signal(SessionState::default);
    use_context_provider(|| session);
    rsx! { {children} }
}
