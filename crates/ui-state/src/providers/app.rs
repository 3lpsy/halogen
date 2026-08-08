//! The provider composition root: `AppProviders` (composes every provider
//! around the app tree) + the shared `LoadingSplash` boot splash.

use dioxus::prelude::*;

use super::{
    AccountsProvider, ConfigProvider, ConnectionStateProvider, DiscoverStateProvider,
    DownloadStateProvider, EpisodeStateProvider, HistoryStateProvider, PlaybackStateProvider,
    PlayerProvider, PlaylistStateProvider, PodcastStateProvider, SessionStateProvider,
    ToastProvider, WebviewMediaBridge, WorkerProvider,
};
/// Boot splash shown by a provider while its async load resolves, so downstream
/// guards never see a transient default. The one shared copy used by
/// `AccountsProvider` and `ConfigProvider`.
#[component]
pub fn LoadingSplash() -> Element {
    rsx! {
        div { class: "min-h-screen flex items-center justify-center bg-background",
            div { class: "text-muted", "Loading…" }
        }
    }
}

/// Composes all providers around the app tree.
///
/// `AccountsProvider` is the device-global outer layer (the account registry +
/// active user); it never remounts. Everything inside it is the **per-user data
/// subtree**, which `AccountsProvider` keys on the active user id so a hot-swap
/// remounts it cleanly.
///
/// Order within the subtree matters: `ConfigProvider` loads + gates a splash and
/// provides the active user's `Signal<ClientConfig>`; `EpisodeStateProvider` provides
/// `Signal<EpisodeState>` and `DownloadStateProvider` the `Signal<DownloadState>`
/// (both worker-written, so they sit above the worker); `WorkerProvider` (needs
/// all of them) spawns the sync worker under the active user's namespace and
/// provides the `Coroutine<Command>` dispatch handle.
#[component]
pub fn AppProviders(children: Element) -> Element {
    // Register the embedded-server URL resolver before ANY config load: the
    // config store overlays the live loopback URL onto Embedded configs at
    // load time, and `ConfigProvider`/login/switch all load through it. A
    // no-op on web / feature-off builds (stub facade).
    use_hook(|| crate::embedded::install());

    rsx! {
        AccountsProvider {
            ConfigProvider {
                // Native-webview media plumbing (local-audio file serving + the
                // authed server media proxy); a pass-through on web/renderless.
                // Inside ConfigProvider (peeks server/token), inside the keyed
                // per-user subtree so a switch re-registers under the new user.
                WebviewMediaBridge {
                EpisodeStateProvider {
                    PodcastStateProvider {
                        PlaylistStateProvider {
                            PlaybackStateProvider {
                                HistoryStateProvider {
                                    DownloadStateProvider {
                                        ConnectionStateProvider {
                                            SessionStateProvider {
                                                ToastProvider {
                                                    WorkerProvider {
                                                        PlayerProvider {
                                                            DiscoverStateProvider {
                                                                // `children` includes the global `ToastContainer`,
                                                                // mounted by the app shell (`app::App`) so it sits
                                                                // inside the toast context but the `ToastContainer`
                                                                // widget stays in ui-widgets (above this layer).
                                                                {children}
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                }
            }
        }
    }
}
