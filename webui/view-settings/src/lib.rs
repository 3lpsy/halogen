//! Settings, split into sub-pages (one per logical group). `/settings` is a menu page, a list of links to the per-group
//! sub-pages (accounts, server, podcasts, playback, downloads, UI), plus the direct links that were already standalone
//! pages (device logs, cache control). Every control the old single-page Settings carried lives on exactly one
//! sub-page.
mod components {
    pub use halogen_webui_component_widgets::{
        BackButton, DownloadPrefsForm, FormSubmit, InputField, PlaybackPrefsForm, SettingsRow,
        ToggleField,
    };
}

pub mod accounts;
pub mod add_embedded_user;
pub mod downloads;
pub mod playback;
pub mod podcasts;
pub mod server;
pub mod ui;

use dioxus::prelude::*;

use crate::components::BackButton;
use halogen_webui_component_icons::{
    DocumentText, Download, Gear, GlobeAlt, Play, Podcast, Trash, User,
};
use halogen_webui_hooks::{use_config, use_is_admin};

/// Shared shell for the settings sub-pages: a pinned back-button row over the
/// scrollable moved section content.
#[component]
pub(crate) fn SettingsSubpage(title: String, children: Element) -> Element {
    rsx! {
        div { class: "flex flex-col h-full overflow-hidden",
            div { class: "p-2", BackButton {} }
            div { class: "flex-1 overflow-y-auto overflow-x-hidden px-2 pb-2 space-y-8",
                h1 { class: "text-3xl font-bold mb-2", "{title}" }
                {children}
            }
        }
    }
}

/// One link row on the settings menu (icon + label, `Menu`-page styling).
#[component]
fn SettingsLink(
    label: String,
    to: String,
    #[props(default)] danger: bool,
    children: Element,
) -> Element {
    let class = if danger {
        "flex items-center gap-3 px-3 py-3 rounded-lg text-error hover:bg-sidebar-hover transition-colors"
    } else {
        "flex items-center gap-3 px-3 py-3 rounded-lg text-foreground hover:bg-sidebar-hover transition-colors"
    };
    rsx! {
        Link { to, class,
            {children}
            span { class: "font-medium", "{label}" }
        }
    }
}

/// The `/settings` menu page: links to the per-group sub-pages, in the same
/// order the groups appeared on the old single page.
#[component]
pub fn Settings() -> Element {
    let is_admin = use_is_admin();
    let config = use_config();
    let embedded = config().server_kind.is_embedded();

    rsx! {
        div { class: "p-2",
            h1 { class: "text-3xl font-bold mb-6", "Settings" }
            nav { class: "flex flex-col gap-1",
                SettingsLink { label: "Accounts", to: "/settings/accounts",
                    User { class: "w-5 h-5" }
                }
                SettingsLink { label: "Server", to: "/settings/server",
                    GlobeAlt { class: "w-5 h-5" }
                }
                // OPML import/export is admin-only (matches the old page's gate).
                if is_admin() {
                    SettingsLink { label: "Podcasts", to: "/settings/podcasts",
                        Podcast { class: "w-5 h-5" }
                    }
                }
                SettingsLink { label: "Playback", to: "/settings/playback",
                    Play { class: "w-5 h-5" }
                }
                // Device-download chunking prefs — meaningless in embedded mode
                // (episodes download into the built-in server, not to a separate
                // device store).
                if !embedded {
                    SettingsLink { label: "Downloads", to: "/settings/downloads",
                        Download { class: "w-5 h-5" }
                    }
                }
                SettingsLink { label: "UI", to: "/settings/ui",
                    Gear { class: "w-5 h-5" }
                }
                // Standalone pages the old Settings only linked to — link straight
                // there, no sub-page wrapper needed.
                SettingsLink { label: "Device Logs", to: "/logs/device",
                    DocumentText { class: "w-5 h-5" }
                }
                // Danger zone: the granular delete/clear actions live on the
                // standalone Cache Control page.
                SettingsLink {
                    label: "Delete local data",
                    to: "/cache-control",
                    danger: true,
                    Trash { class: "w-5 h-5" }
                }
            }
        }
    }
}
