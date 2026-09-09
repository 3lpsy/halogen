use dioxus::prelude::*;
use halogen_webui_app_state::DiscoverMode;
use halogen_webui_hooks::{
    infinite_scroll_body, use_config, use_discover_store, use_dom_stream, use_sync_status,
};
use halogen_wire::DiscoverProviderInfo;

use crate::{
    controls::DiscoverControls,
    loading::load_page,
    rows::{EpisodeRow, PodcastRow},
};

#[component]
pub fn Discover() -> Element {
    let config = use_config();
    let sync = use_sync_status();
    let mut store = use_discover_store();
    let mut providers = use_signal(Vec::<DiscoverProviderInfo>::new);
    let mut provider_loading = use_signal(|| false);
    let mut provider_error = use_signal(|| false);
    let mut retry = use_signal(|| 0u32);
    use_effect(move || {
        let _ = retry();
        if sync().is_offline() || !providers.peek().is_empty() || *provider_loading.peek() {
            return;
        }
        let Some(client) = config.read().api_client() else {
            return;
        };
        provider_loading.set(true);
        spawn(async move {
            match client.discover_providers().await {
                Ok(data) => {
                    providers.set(data.providers);
                    provider_error.set(false);
                }
                Err(_) => provider_error.set(true),
            }
            provider_loading.set(false);
        });
    });
    use_dom_stream(
        || infinite_scroll_body("discover-scroll", "discover-sentinel"),
        move |visible: bool| {
            let state = store.peek();
            let is_ready = visible
                && state.searched
                && !state.loading
                && state.failure.is_none()
                && state.page.as_ref().is_some_and(|page| page.has_more);
            drop(state);
            if is_ready && !sync.peek().is_offline() {
                load_page(store, config);
            }
        },
    );
    let restore = store.peek().scroll_top.max(0.0);
    use_dom_stream(
        move || {
            format!(
                r#"
        const root = document.getElementById('discover-scroll');
        if (root) {{
            requestAnimationFrame(() => {{ if (!signal.aborted) root.scrollTop = {restore}; }});
            root.addEventListener('scroll', () => dioxus.send(root.scrollTop), {{signal, passive:true}});
        }}
    "#
            )
        },
        move |top: f64| {
            if top.is_finite() {
                store.write().scroll_top = top;
            }
        },
    );
    use_drop(move || {
        let mut state = store.write();
        state.generation = state.generation.wrapping_add(1);
        state.loading = false;
    });
    let state = store.read().clone();
    let offline = sync().is_offline();
    let empty = match state.mode {
        DiscoverMode::Podcasts => state.results.is_empty(),
        DiscoverMode::Episodes => state.episodes.is_empty(),
    };
    rsx! {
        div { id: "discover-scroll", class: "h-full overflow-y-auto overflow-x-hidden",
            div { class: "p-2 space-y-3 max-w-4xl mx-auto",
                div { class: "flex items-center justify-between gap-2",
                    h1 { class: "text-2xl font-bold", "Discover" }
                    Link { to: "/podcasts/create", class: "btn btn-sm btn-ghost", "Add by URL" }
                }
                if offline { div { class: "alert alert-warning text-sm", "Discover needs an internet connection." } }
                if provider_error() {
                    div { class: "alert alert-error text-sm",
                        "Couldn't load search providers."
                        button { class: "btn btn-sm", disabled: offline || provider_loading(), onclick: move |_| retry += 1, "Retry" }
                    }
                }
                DiscoverControls { providers: providers(), offline }
                for error in &state.errors {
                    p { class: "text-sm text-warning", role: "status", "{error.message}" }
                }
                if state.mode == DiscoverMode::Podcasts {
                    for item in state.results { PodcastRow { key: "{item.id}", item } }
                } else {
                    for item in state.episodes { EpisodeRow { key: "{item.id}", item } }
                }
                if let Some(error) = state.failure {
                    div { class: "alert alert-error text-sm flex-wrap", role: "alert",
                        span { "Search failed: {error}" }
                        button { class: "btn btn-sm", disabled: offline, onclick: move |_| load_page(store, config), "Retry" }
                        button { class: "btn btn-sm btn-ghost", disabled: offline,
                            onclick: move |_| {
                                let current = store.peek().clone();
                                store.write().reset(current.query, current.mode, current.providers);
                                load_page(store, config);
                            }, "Restart search"
                        }
                    }
                } else if state.loading {
                    div { class: "flex justify-center p-4", role: "status", "aria-label": "Loading results",
                        span { class: "loading loading-spinner" }
                    }
                } else if empty && state.errors.is_empty() {
                    p { class: "text-muted py-6 text-center",
                        if state.searched { "No matching results from the selected providers." }
                        else { "Search for podcasts or episodes." }
                    }
                }
                if let Some(page) = state.page {
                    if !page.has_more && page.result_limit > 0 && state.searched {
                        p { class: "text-xs text-muted py-4", "End of this search. Provider limit: up to {page.result_limit} results." }
                    } else if page.has_more && !state.loading {
                        button { class: "btn btn-ghost btn-sm", disabled: offline, onclick: move |_| load_page(store, config), "Load more" }
                    }
                }
                div { id: "discover-sentinel", class: "h-4" }
            }
        }
    }
}
