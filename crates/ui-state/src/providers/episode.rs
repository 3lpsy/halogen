use dioxus::prelude::*;

use halogen_ui_appstate::EpisodeState;

/// Provides the shared `Signal<EpisodeState>` (the episode pool; the other domains
/// are sibling slices). The sync worker is the only writer; pages/components read it
/// (via `hooks::use_episodes`).
///
/// Reactivity contract:
/// - **Single writer**: only the worker's `publish()` ever sets this signal.
///   Components must never write it — optimistic updates go through the
///   dispatch coroutine so the worker stays the source of truth.
/// - **Publish flood**: `Signal::set` notifies unconditionally, so each publish
///   re-renders every component that `.read()`s this signal in its body, even
///   if nothing it uses changed. Narrow consumers should go through a
///   `PartialEq`-gated memo slice (exemplar: `hooks::use_sync_status`).
/// - **`EpisodeState: PartialEq` is load-bearing** for components that take this
///   signal as a `ReadSignal<EpisodeState>` prop — see the derive comment on
///   `EpisodeState` (appstate/state.rs) for the re-render-loop failure mode.
#[component]
pub fn EpisodeStateProvider(children: Element) -> Element {
    let state = use_signal(EpisodeState::default);
    use_context_provider(|| state);
    rsx! { {children} }
}
