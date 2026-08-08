use dioxus::prelude::*;

use halogen_ui_appstate::HistoryState;

/// Provides the shared `Signal<HistoryState>` (the `/playbacks` History paging
/// cursor). The sync worker is the only writer; components read it via
/// [`hooks::use_history`](crate::hooks::use_history).
///
/// A separate signal from `EpisodeState`, so advancing the cursor as History pages
/// re-renders only the History page-ahead effect, not every `EpisodeState` subscriber.
/// Follows the same shape as `EpisodeStateProvider`.
#[component]
pub fn HistoryStateProvider(children: Element) -> Element {
    let history = use_signal(HistoryState::default);
    use_context_provider(|| history);
    rsx! { {children} }
}
