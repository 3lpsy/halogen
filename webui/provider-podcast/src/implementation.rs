use dioxus::prelude::*;

use halogen_webui_app_state::PodcastState;

/// Provide worker-written podcast and auto-playlist state separately from episodes so podcast mutations notify only
/// podcast consumers.
#[component]
pub fn PodcastStateProvider(children: Element) -> Element {
    let podcasts = use_signal(PodcastState::default);
    use_context_provider(|| podcasts);
    rsx! { {children} }
}
