//! Settings → UI (`/settings/ui`): font size plus the dock/navigation and
//! swipe-action configuration links, moved verbatim from the old single-page
//! Settings. The dock + swipe editors keep their existing routes
//! (`/settings/dock`, `/settings/configure-swipes`); this page just links there.

use dioxus::prelude::*;

use super::SettingsSubpage;
use crate::components::SettingsRow;
use halogen_webui_config::config_actions::persist_config;
use halogen_webui_config::{FontSize, SelectEnum};
use halogen_webui_hooks::use_config;

#[component]
pub fn SettingsUi() -> Element {
    let config = use_config();
    let nav = use_navigator();
    let mut font_size = use_signal(|| config().font_size);

    // Persist font size whenever it changes.
    use_effect(move || {
        let fs = *font_size.read();
        persist_config(config, move |c| c.font_size = fs);
    });

    rsx! {
        SettingsSubpage { title: "UI",
            div { class: "space-y-4",
                div { class: "space-y-2",
                    label { class: "text-muted", "Font Size" }
                    select {
                        "aria-label": "Font size",
                        class: "select select-bordered w-full",
                        value: "{font_size().as_str()}",
                        onchange: move |e| {
                            font_size.set(FontSize::from_str_or_default(&e.value()));
                        },
                        for size in FontSize::ALL {
                            option {
                                value: "{size.as_str()}",
                                selected: "{font_size() == size}",
                                "{size.label()}"
                            }
                        }
                    }
                }
                // Reorder + show/hide the dock, sidebar, and menu items.
                SettingsRow { label: "Dock & navigation",
                    button {
                        class: "btn btn-primary btn-sm",
                        onclick: move |_| { nav.push("/settings/dock"); },
                        "Configure dock"
                    }
                }
                // Per-page episode swipe-action customization.
                SettingsRow { label: "Episode swipe actions",
                    button {
                        class: "btn btn-primary btn-sm",
                        onclick: move |_| { nav.push("/settings/configure-swipes"); },
                        "Customize Swipes"
                    }
                }
            }
        }
    }
}
