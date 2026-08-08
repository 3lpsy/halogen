//! Settings → Playback (`/settings/playback`): the playback preferences form,
//! moved verbatim from the old single-page Settings.

use dioxus::prelude::*;

use super::SettingsSubpage;
use crate::components::PlaybackPrefsForm;
use halogen_ui_config::config_actions::persist_config;
use halogen_ui_state::hooks::use_config;

#[component]
pub fn SettingsPlayback() -> Element {
    let config = use_config();
    let playback_prefs = use_signal(|| config().playback_prefs.clone());

    // Persist playback prefs whenever they change.
    use_effect(move || {
        let prefs = playback_prefs.read().clone(); // subscribe to prefs only
        persist_config(config, move |c| c.playback_prefs = prefs);
    });

    rsx! {
        SettingsSubpage { title: "Playback",
            PlaybackPrefsForm {
                prefs: playback_prefs,
                embedded: config().server_kind.is_embedded(),
            }
        }
    }
}
