use dioxus::prelude::*;

use halogen_ui_appstate::DiscoverState;

/// Provides the shared `Signal<DiscoverState>`. Not part of `EpisodeState` — the
/// sync worker is the only writer of `EpisodeState`, and Discover never persists.
/// The [`DiscoverState`] type itself lives in `halogen-ui-appstate`.
#[component]
pub fn DiscoverStateProvider(children: Element) -> Element {
    let store = use_signal(DiscoverState::default);
    use_context_provider(|| store);
    rsx! { {children} }
}
