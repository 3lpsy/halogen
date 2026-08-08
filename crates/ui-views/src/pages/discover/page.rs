//! Discover — online-only podcast search.
//!
//! A search bar + provider-filter chips + a bare result list. Results come from
//! `/api/v1/discover/*` (the server proxies the external providers; the UI never
//! calls them directly). The page is the sole writer of the ephemeral
//! `DiscoverState`; the detail page (`DiscoverDetail`) reads it.

use dioxus::prelude::*;
use halogen_wire::{
    DiscoverProvider, DiscoverProviderInfo, DiscoverResultItem, DiscoverSearchParams,
};

use crate::Route;
use halogen_ui_appstate::DiscoverState;
use halogen_ui_config::ClientConfig;
use halogen_ui_config::config_actions::persist_config;
use halogen_ui_state::hooks::{use_config, use_discover_store, use_sync_status, use_toast};

/// The providers to query: available ones the user hasn't toggled off.
fn enabled_providers(
    config: &ClientConfig,
    infos: &[DiscoverProviderInfo],
) -> Vec<DiscoverProvider> {
    infos
        .iter()
        .filter(|i| i.available && !config.disabled_discover_providers.contains(&i.id))
        .map(|i| i.id)
        .collect()
}

#[component]
pub fn Discover() -> Element {
    let config = use_config();
    let toast = use_toast();
    let sync_status = use_sync_status();
    let mut store = use_discover_store();

    // Seed from the last search so going to a detail and back restores the view.
    let mut query = use_signal(|| store.peek().query.clone());
    let mut loading = use_signal(|| false);
    let mut searched = use_signal(|| !store.peek().results.is_empty());
    let mut providers = use_signal(Vec::<DiscoverProviderInfo>::new);
    // In-flight guard: this effect subscribes to `sync_status()`/`config`, so a
    // change while the fetch is in flight would spawn a second `discover_providers()`
    // — the empty-`providers` check alone doesn't cover that window.
    let mut in_flight = use_signal(|| false);
    // A failed provider fetch must not silently disable Search forever: pre-fix,
    // one transient error left `providers` empty with nothing re-triggering the
    // effect, and the submit guard then swallowed every search with no feedback.
    let mut provider_error = use_signal(|| false);
    let mut provider_retry = use_signal(|| 0u32);

    let offline = sync_status().is_offline();

    // Fetch the provider list once (for the toggle chips). Re-checks connectivity
    // inside so it fires when the app comes online; the `peek` guard makes the
    // successful fetch a one-shot. Subscribes to `provider_retry` so the error
    // alert's Retry button re-fires it.
    use_effect(move || {
        let _ = provider_retry();
        if sync_status().is_offline() || !providers.peek().is_empty() || *in_flight.peek() {
            return;
        }
        let Some(client) = config.read().api_client() else {
            return;
        };
        in_flight.set(true);
        spawn(async move {
            match client.discover_providers().await {
                Ok(data) => {
                    provider_error.set(false);
                    providers.set(data.providers);
                }
                Err(_) => provider_error.set(true),
            }
            in_flight.set(false);
        });
    });

    let results = store.read().results.clone();

    rsx! {
        div { class: "h-full overflow-y-auto overflow-x-hidden",
        div { class: "p-2 space-y-3",
            div { class: "flex items-center justify-between gap-2",
                h1 { class: "text-2xl font-bold", "Discover" }
                // Manual add-by-URL (moved here from Settings).
                Link {
                    to: Route::PodcastCreate {},
                    class: "btn btn-sm btn-ghost",
                    "Add by URL"
                }
            }

            if offline {
                div { class: "alert alert-warning text-sm",
                    "Discover needs an internet connection." }
            }

            if provider_error() && !offline {
                div { class: "alert alert-error text-sm flex items-center justify-between gap-2",
                    span { "Couldn't load the search providers." }
                    button {
                        class: "btn btn-sm",
                        onclick: move |_| provider_retry += 1,
                        "Retry"
                    }
                }
            }

            // Search bar.
            form {
                class: "flex gap-2",
                onsubmit: move |e| {
                    e.prevent_default();
                    let q = query.peek().trim().to_string();
                    if q.len() < 2 {
                        return;
                    }
    // Don't search until the provider list has loaded: an empty
                    // `providers` means "still loading" (silently wait) — or a
                    // FAILED load, which deserves feedback instead of a dead
                    // search button.
                    if providers.peek().is_empty() {
                        if *provider_error.peek() {
                            toast.warn("Search providers failed to load — use Retry above.");
                        }
                        return;
                    }
                    let enabled = enabled_providers(&config.read(), &providers.read());
                    if enabled.is_empty() {
                        toast.warn("Enable at least one provider");
                        return;
                    }
                    let Some(client) = config.read().api_client() else {
                        toast.error("No server configured");
                        return;
                    };
                    loading.set(true);
                    searched.set(true);
                    spawn(async move {
                        let params = DiscoverSearchParams { q: q.clone(), providers: Some(enabled) };
                        match client.discover_search(params).await {
                            Ok(data) => {
                                for err in &data.errors {
                                    toast.error(format!("{} search failed", err.provider.label()));
                                }
                                store.set(DiscoverState { query: q, results: data.items });
                            }
                            Err(e) => {
                                toast.error(format!("Search failed: {e}"));
                            }
                        }
                        loading.set(false);
                    });
                },
                input {
                    "aria-label": "Search podcasts",
                    r#type: "search",
                    placeholder: "Search podcasts…",
                    class: "input input-bordered flex-1 min-w-0",
                    value: "{query}",
                    disabled: offline,
                    oninput: move |e| query.set(e.value()),
                }
                button {
                    r#type: "submit",
                    class: "btn btn-primary",
                    // Disabled while the provider list is still loading (empty +
                    // online): submitting then would search with no enabled
                    // providers. `offline` already disables it in the offline case.
                    disabled: offline || loading() || (!offline && providers.read().is_empty()),
                    "Search"
                }
            }

            // Provider filter chips.
            if !providers.read().is_empty() {
                div { class: "flex flex-wrap gap-2",
                    for info in providers.read().iter().cloned() {
                        {
                            let pid = info.id;
                            let on = info.available
                                && !config.read().disabled_discover_providers.contains(&pid);
                            rsx! {
                                button {
                                    r#type: "button",
                                    class: if on { "btn btn-xs btn-primary" } else { "btn btn-xs btn-outline opacity-60" },
                                    disabled: !info.available,
                                    onclick: move |_| {
                                        persist_config(config, move |cfg| {
                                            if let Some(pos) = cfg
                                                .disabled_discover_providers
                                                .iter()
                                                .position(|x| *x == pid)
                                            {
                                                cfg.disabled_discover_providers.remove(pos);
                                            } else {
                                                cfg.disabled_discover_providers.push(pid);
                                            }
                                        });
                                    },
                                    "{info.label}"
                                }
                            }
                        }
                    }
                }
            }

            // Results.
            if loading() {
                div { class: "flex justify-center py-8",
                    span { class: "loading loading-spinner loading-lg" } }
            } else if results.is_empty() {
                p { class: "text-center text-muted py-8",
                    if searched() { "No podcasts found." } else { "Search for podcasts to discover." }
                }
            } else {
                div { class: "rounded-lg border border-base-200 overflow-hidden",
                    for item in results.iter().cloned() {
                        DiscoverItem { key: "{item.id}", item }
                    }
                }
            }
        }
        }
    }
}

/// A bare result row: provider badge, title, author, a 3-line description, and
/// the feed URL. No artwork, no swipe actions. Tapping opens the detail page.
#[component]
fn DiscoverItem(item: DiscoverResultItem) -> Element {
    let nav = use_navigator();
    let id = item.id.clone();
    rsx! {
        button {
            class: "w-full text-left p-4 border-b border-base-200 last:border-b-0 hover:bg-base-200/50 flex flex-col gap-1",
            onclick: move |_| {
                nav.push(Route::DiscoverDetail { id: id.clone() });
            },
            div { class: "flex items-center gap-2 min-w-0",
                span { class: "badge badge-sm badge-outline shrink-0", "{item.provider.label()}" }
                // Result-card title — not a heading (h1 "Discover" is the only one on
                // the page; an h3 here skips h2 for Lighthouse). Styled identically.
                div { class: "font-semibold truncate", "{item.title}" }
            }
            if let Some(author) = item.author.clone() {
                p { class: "text-xs text-muted truncate", "{author}" }
            }
            if !item.description.is_empty() {
                p { class: "text-sm text-muted line-clamp-3", "{item.description}" }
            }
            p { class: "text-xs text-muted truncate", "{item.feed_url}" }
        }
    }
}
