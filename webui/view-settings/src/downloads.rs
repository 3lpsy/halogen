//! Settings → Downloads (`/settings/downloads`): the download preferences form,
//! moved verbatim from the old single-page Settings.

use dioxus::prelude::*;

use super::SettingsSubpage;
use crate::components::DownloadPrefsForm;
use halogen_webui_config::config_actions::persist_config;
use halogen_webui_hooks::use_config;

#[component]
pub fn SettingsDownloads() -> Element {
    let config = use_config();
    let download_prefs = use_signal(|| config().download_prefs);

    // Persist download prefs whenever they change.
    use_effect(move || {
        let prefs = *download_prefs.read(); // subscribe to prefs only
        persist_config(config, move |c| c.download_prefs = prefs);
    });

    // Embedded mode has no device downloads to tune (the menu link is hidden;
    // this covers a stale deep link). Branch AFTER the hooks above — a
    // conditional early return before them would break hook ordering.
    if config().server_kind.is_embedded() {
        return rsx! {
            SettingsSubpage { title: "Downloads",
                p { class: "text-sm text-muted p-3",
                    "Not applicable with the embedded server — episodes download into the built-in server (Settings → Server)."
                }
            }
        };
    }

    rsx! {
        SettingsSubpage { title: "Downloads",
            DownloadPrefsForm { prefs: download_prefs }
        }
    }
}
