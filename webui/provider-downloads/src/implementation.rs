use dioxus::prelude::*;

use halogen_webui_app_state::DownloadState;

/// Provides the shared `Signal<DownloadState>` (per-episode device/server download state). The sync worker is the only
/// writer; components read it via [`hooks::use_downloads`](halogen_webui_hook_context::use_downloads). A separate
/// signal from `EpisodeState`, so a ~1 Hz download-progress byte re-renders only download consumers (the row badges,
/// the Downloads list), not every `EpisodeState` subscriber. Follows the same shape as `EpisodeStateProvider`.
#[component]
pub fn DownloadStateProvider(children: Element) -> Element {
    let downloads = use_signal(DownloadState::default);
    use_context_provider(|| downloads);
    rsx! { {children} }
}
