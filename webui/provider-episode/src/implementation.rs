use dioxus::prelude::*;

use halogen_webui_app_state::EpisodeState;

/// Only the sync worker publishes episode state; UI mutations go through its command coroutine. Signal writes notify
/// unconditionally, so narrow readers need PartialEq memos. EpisodeState must remain PartialEq to prevent ReadSignal
/// prop memoization from causing render loops.
#[component]
pub fn EpisodeStateProvider(children: Element) -> Element {
    let state = use_signal(EpisodeState::default);
    use_context_provider(|| state);
    rsx! { {children} }
}
