use dioxus::prelude::*;
use halogen_webui_app_state::DiscoverMode;
use halogen_webui_config::config_actions::persist_config;
use halogen_webui_hooks::{use_config, use_discover_store};
use halogen_wire::DiscoverProviderInfo;

use crate::loading::load_page;

#[component]
pub fn DiscoverControls(providers: Vec<DiscoverProviderInfo>, offline: bool) -> Element {
    let config = use_config();
    let mut store = use_discover_store();
    let mode = store.read().mode;
    let enabled: Vec<_> = providers
        .iter()
        .filter(|p| {
            p.available
                && (mode == DiscoverMode::Podcasts
                    || p.id == halogen_wire::DiscoverProvider::Itunes)
                && !config.read().disabled_discover_providers.contains(&p.id)
        })
        .map(|p| p.id)
        .collect();
    let enabled_submit = enabled.clone();
    let enabled_input = enabled.clone();
    let enabled_mode = enabled.clone();
    let state = store.read().clone();
    rsx! {
        form { class: "flex gap-2 flex-wrap",
            onsubmit: move |event| {
                event.prevent_default();
                if offline || enabled_submit.is_empty() { return; }
                let current = store.peek().clone();
                store.write().reset(current.query, current.mode, enabled_submit.clone());
                load_page(store, config);
            },
            div { class: "flex gap-2 flex-1 basis-72 min-w-0",
                input { r#type: "search", class: "input input-bordered flex-1 min-w-0",
                    "aria-label": if state.mode == DiscoverMode::Podcasts { "Search podcasts" } else { "Search episodes" },
                    placeholder: if state.mode == DiscoverMode::Podcasts { "Search podcasts" } else { "Search episodes" },
                    maxlength: "256", value: "{state.query}", disabled: offline,
                    oninput: move |event| {
                        let mode = store.peek().mode;
                        store.write().reset(event.value(), mode, enabled_input.clone());
                    }
                }
                select { class: "select select-bordered shrink-0 w-36", "aria-label": "Search by",
                    value: if state.mode == DiscoverMode::Podcasts { "podcast" } else { "episode" },
                    onchange: move |event| {
                        let mode = if event.value() == "episode" { DiscoverMode::Episodes } else { DiscoverMode::Podcasts };
                        let query = store.peek().query.clone();
                        store.write().reset(query, mode, enabled_mode.clone());
                    },
                    option { value: "podcast", "By Podcast" }
                    option { value: "episode", "By Episode" }
                }
            }
            button { r#type: "submit", class: "btn btn-primary", disabled: offline || enabled.is_empty() || state.loading || state.query.trim().chars().count() < 2, "Search" }
        }
        div { class: "flex flex-wrap gap-2",
            for info in providers.iter().cloned() {
                {
                    let pid = info.id;
                    let supported = mode == DiscoverMode::Podcasts || pid == halogen_wire::DiscoverProvider::Itunes;
                    let on = info.available && supported && !config.read().disabled_discover_providers.contains(&pid);
                    let all_providers = providers.clone();
                    rsx! { button { r#type: "button", class: if on { "btn btn-xs btn-primary" } else { "btn btn-xs btn-outline" },
                        disabled: !info.available || !supported, "aria-pressed": on.to_string(),
                        onclick: move |_| {
                            persist_config(config, move |cfg| {
                                if let Some(index) = cfg.disabled_discover_providers.iter().position(|p| *p == pid) { cfg.disabled_discover_providers.remove(index); }
                                else { cfg.disabled_discover_providers.push(pid); }
                            });
                            let next = all_providers.iter().filter(|p| p.available && (mode == DiscoverMode::Podcasts || p.id == halogen_wire::DiscoverProvider::Itunes) && !config.read().disabled_discover_providers.contains(&p.id)).map(|p| p.id).collect();
                            let current = store.peek().clone();
                            store.write().reset(current.query, current.mode, next);
                        }, "{info.label}"
                    } }
                }
            }
        }
        if mode == DiscoverMode::Episodes { p { class: "text-xs text-muted", "gpodder supports podcast search only." } }
        if !providers.is_empty() && enabled.is_empty() { p { class: "text-sm text-warning", if mode == DiscoverMode::Episodes { "Enable iTunes to search episodes." } else { "Enable at least one provider." } } }
    }
}
