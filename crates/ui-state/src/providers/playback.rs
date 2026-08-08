use dioxus::prelude::*;

use halogen_ui_appstate::PlaybackState;

/// Provides the shared `Signal<PlaybackState>` (the per-episode playback cursors
/// overlay). The sync worker is the only writer; components read it via
/// [`hooks::use_playbacks`](crate::hooks::use_playbacks).
///
/// A separate signal from `EpisodeState`, so a cursor save (seek / mark-played) or a
/// History page re-renders only playback consumers (row progress bars + played markers,
/// the History list), not every `EpisodeState` subscriber. Follows the same shape as
/// `EpisodeStateProvider`.
#[component]
pub fn PlaybackStateProvider(children: Element) -> Element {
    let playbacks = use_signal(PlaybackState::default);
    use_context_provider(|| playbacks);
    rsx! { {children} }
}
