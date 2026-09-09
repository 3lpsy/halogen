//! The provider composition root: `AppProviders` (composes every provider
//! around the app tree) + the shared `LoadingSplash` boot splash.

use dioxus::prelude::*;

use halogen_webui_provider_accounts::*;
use halogen_webui_provider_config::*;
use halogen_webui_provider_connection::*;
use halogen_webui_provider_discover::*;
use halogen_webui_provider_downloads::*;
use halogen_webui_provider_episode::*;
use halogen_webui_provider_history::*;
use halogen_webui_provider_playback::*;
use halogen_webui_provider_player::*;
use halogen_webui_provider_playlist::*;
use halogen_webui_provider_podcast::*;
use halogen_webui_provider_session::*;
use halogen_webui_provider_sync::*;
use halogen_webui_provider_toast::*;
use halogen_webui_provider_webview_media::*;
/// `AccountsProvider` survives user switches and keys the inner subtree by user ID. Inside, config loads behind a
/// splash, episode/download providers create worker-owned signals, then `WorkerProvider` supplies the namespaced sync
/// coroutine.
#[component]
pub fn AppProviders(children: Element) -> Element {
    // Register the local-runtime URL resolver before ANY config load: the
    // config store overlays the live loopback URL onto Embedded configs at
    // load time, and `ConfigProvider`/login/switch all load through it. A
    // no-op on web / feature-off builds (stub facade).
    use_hook(halogen_webui_provider_local::embedded::install);

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
