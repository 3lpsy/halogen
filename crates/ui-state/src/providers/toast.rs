use dioxus::prelude::*;

use halogen_ui_toast::ToastQueue;

/// Provides the global `Signal<ToastQueue>`.
///
/// Mounted **above** `WorkerProvider` (the worker raises toasts) and around the
/// `ToastContainer` (which renders them). Follows the same shape as
/// `EpisodeStateProvider`.
#[component]
pub fn ToastProvider(children: Element) -> Element {
    let store = use_signal(ToastQueue::default);
    use_context_provider(|| store);
    rsx! { {children} }
}
