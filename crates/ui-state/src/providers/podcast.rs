use dioxus::prelude::*;

use halogen_ui_appstate::PodcastState;

/// Provides the shared `Signal<PodcastState>` (the podcast pool + per-podcast
/// auto-playlist config). The sync worker is the only writer; components read it
/// via [`hooks::use_podcasts`](crate::hooks::use_podcasts).
///
/// A separate signal from `EpisodeState`, so a podcast cache write (paged list / detail
/// fetch) or an auto-playlist edit re-renders only podcast consumers, not every
/// `EpisodeState` subscriber. Follows the same shape as `EpisodeStateProvider`.
#[component]
pub fn PodcastStateProvider(children: Element) -> Element {
    let podcasts = use_signal(PodcastState::default);
    use_context_provider(|| podcasts);
    rsx! { {children} }
}
