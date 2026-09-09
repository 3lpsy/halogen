use dioxus::prelude::*;

use halogen_webui_app_state::PlaybackState;

/// Provide worker-written playback cursors separately from episodes so cursor saves and history pages notify playback
/// consumers without rerendering unrelated episode subscribers.
#[component]
pub fn PlaybackStateProvider(children: Element) -> Element {
    let playbacks = use_signal(PlaybackState::default);
    use_context_provider(|| playbacks);
    rsx! { {children} }
}
