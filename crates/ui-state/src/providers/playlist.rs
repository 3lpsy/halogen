use dioxus::prelude::*;

use halogen_ui_appstate::PlaylistState;

/// Provides the shared `Signal<PlaylistState>` (the playlists pool + queue
/// resolution). The sync worker is the only writer; components read it via
/// [`hooks::use_playlists`](crate::hooks::use_playlists).
///
/// A separate signal from `EpisodeState`, so a playlist mutation (add/remove/reorder,
/// default resolution) re-renders only playlist/queue consumers, not every
/// `EpisodeState` subscriber. Follows the same shape as `EpisodeStateProvider`.
#[component]
pub fn PlaylistStateProvider(children: Element) -> Element {
    let playlists = use_signal(PlaylistState::default);
    use_context_provider(|| playlists);
    rsx! { {children} }
}
