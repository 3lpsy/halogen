use std::future::Future;
use std::pin::Pin;

use dioxus::prelude::*;
use halogen_api::ApiClient;

use halogen_ui_config::ClientConfig;

/// Lazy / deep-link fetch for a detail page.
///
/// When `present()` is false (the entity isn't in the pool yet — a cold-store
/// deep link, or a list that hasn't paged this far), run `load` once to fetch +
/// cache it. Returns a `load_failed` signal so the page can distinguish "loading"
/// from "not found" instead of crying "not found" on every transient/offline miss.
///
/// `present` is read inside the effect, so it subscribes to whatever it touches
/// (typically `EpisodeState`): once the row lands the effect no-ops, and a later
/// publish (e.g. connectivity returns → pull) re-runs `load`, recovering an
/// offline miss on its own. `load` should build its own client snapshot and
/// return `Err` for "no server" / fetch failure (both surface as `load_failed`).
pub fn use_deep_link_fetch(
    present: impl Fn() -> bool + 'static,
    load: impl Fn() -> Pin<Box<dyn Future<Output = Result<(), String>>>> + 'static,
) -> Signal<bool> {
    let mut load_failed = use_signal(|| false);
    // In-flight guard: the effect re-runs on every `present()` subscription change
    // (typically each `EpisodeState` publish), so without this a burst of publishes
    // before the first fetch resolves would spawn several concurrent loads for the
    // same entity. `peek` so toggling it doesn't itself re-trigger the effect.
    let mut in_flight = use_signal(|| false);
    use_effect(move || {
        if present() || *in_flight.peek() {
            return;
        }
        // A new attempt: clear the previous failure so the UI shows "loading"
        // during a retry rather than a stale "not found".
        load_failed.set(false);
        in_flight.set(true);
        let fut = load();
        spawn(async move {
            let failed = fut.await.is_err();
            load_failed.set(failed);
            in_flight.set(false);
        });
    });
    load_failed
}

/// The placeholder message a detail page shows while its entity isn't in the pool
/// yet, derived from the `load_failed` signal [`use_deep_link_resource`] returns:
/// "loading" until the fetch fails, then an offline-aware "not synced" vs a plain
/// "not found". `noun` is the lowercase entity name, e.g. `"podcast"`.
///
/// Centralizing this keeps the offline-aware variant consistent across
/// podcast/episode/playlist, rather than each page deriving its own placeholder.
pub fn deep_link_placeholder(load_failed: bool, is_offline: bool, noun: &str) -> String {
    if !load_failed {
        format!("Loading {noun}…")
    } else if is_offline {
        format!("You're offline — this {noun} hasn't been synced to this device yet.")
    } else {
        let mut chars = noun.chars();
        let capitalized = match chars.next() {
            Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
            None => String::new(),
        };
        format!("{capitalized} not found.")
    }
}

/// Higher-level deep-link fetch that captures the boilerplate shared by every
/// detail page: the `api_client_or_err` guard (snapshotting `config`) and the
/// `Err(String)` mapping, so a page supplies only its presence check and the
/// fetch-plus-cache body.
///
/// `present` is the same offline-first presence check as
/// [`use_deep_link_fetch`] (read inside the effect, so it subscribes to whatever it
/// touches — typically `EpisodeState`). `fetch` receives a ready [`ApiClient`] and does
/// the per-entity request + cache, returning `Err` (its message) on failure. It's
/// run once on a cold-store miss and re-run on a later publish, recovering offline
/// misses on their own. Returns the `load_failed` signal so the page can show
/// "loading" vs "not found".
pub fn use_deep_link_resource<P, Fut, Fetch>(
    config: Signal<ClientConfig>,
    present: P,
    fetch: Fetch,
) -> Signal<bool>
where
    P: Fn() -> bool + 'static,
    Fut: Future<Output = Result<(), String>> + 'static,
    Fetch: Fn(ApiClient) -> Fut + Copy + 'static,
{
    use_deep_link_fetch(present, move || {
        // Snapshot the config (server URL/token) per attempt; `peek` so building the
        // client never makes the load re-subscribe to config changes.
        let cfg = config.peek().clone();
        Box::pin(async move {
            let client = cfg.api_client_or_err()?;
            fetch(client).await
        })
    })
}
