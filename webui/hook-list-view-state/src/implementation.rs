//! Seed list state from the captured URL query, then persisted ListViews, then defaults. Capture before router startup
//! strips queries; consume once for the matching path. Debounced persistence is separate from auth config and has no
//! reactive readers; replaceState updates the URL without navigation.

use dioxus::prelude::*;

use halogen_webui_config::{ListViewStore, ListViews, StoredListView};
use halogen_webui_hook_latest_wins::use_latest_wins;
use halogen_webui_listview::{
    EpisodeFilter, FilterSpec, ListViewState, OrderDirection, SortField, SortSpec,
};
use halogen_webui_platform::time::sleep_ms;

#[cfg(target_arch = "wasm32")]
thread_local! {
    /// `(path, query)` snapshot of the URL at program start, before the router can
    /// strip the query. Consumed once by the matching list's seed.
    static INITIAL_QUERY: std::cell::RefCell<Option<(String, String)>> =
        const { std::cell::RefCell::new(None) };
}

/// Snapshot the initial URL path + query. MUST be called from `main()` before
/// `dioxus::launch`, while `location.search` still holds any deep-linked/shared
/// query the router is about to normalize away. No-op off wasm.
#[cfg(target_arch = "wasm32")]
pub fn capture_initial_query() {
    if let Some(win) = web_sys::window() {
        let loc = win.location();
        let path = loc.pathname().unwrap_or_default();
        let search = loc.search().unwrap_or_default();
        let qs = search.strip_prefix('?').unwrap_or(&search).to_string();
        INITIAL_QUERY.with(|c| *c.borrow_mut() = Some((path, qs)));
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn capture_initial_query() {}

/// Read a query-param value from the URL captured at startup (before the router
/// normalized it away). Non-consuming — unlike the list-view seed it only borrows,
/// so it's safe for any page to call. `None` off wasm or when the param is absent.
/// Used by `/cache-control` to surface a `?message=…` passed by a failure redirect.
#[cfg(target_arch = "wasm32")]
pub fn read_initial_query_param(name: &str) -> Option<String> {
    INITIAL_QUERY.with(|c| {
        let snap = c.borrow();
        let (_, qs) = snap.as_ref()?;
        for pair in qs.split('&') {
            let mut kv = pair.splitn(2, '=');
            if kv.next() == Some(name) {
                let raw = kv.next().unwrap_or("").replace('+', " ");
                return Some(
                    js_sys::decode_uri_component(&raw)
                        .ok()
                        .and_then(|s| wasm_bindgen::JsValue::from(s).as_string())
                        .unwrap_or(raw),
                );
            }
        }
        None
    })
}

#[cfg(not(target_arch = "wasm32"))]
pub fn read_initial_query_param(_name: &str) -> Option<String> {
    None
}

/// Idle window before a change is persisted to sticky config.
const SAVE_DEBOUNCE_MS: u32 = 400;

fn stored_to_state(s: &StoredListView) -> Option<ListViewState> {
    let field = SortField::from_token(&s.sort_field)?;
    let direction = OrderDirection::from_token(&s.sort_dir).unwrap_or(OrderDirection::Desc);
    let filters = s
        .filters
        .iter()
        .filter_map(|t| EpisodeFilter::from_token(t))
        .collect();
    Some(ListViewState {
        sort: SortSpec { field, direction },
        filters,
        // Search is never restored from sticky storage — only from the URL.
        search: None,
    })
}

fn state_to_stored(s: &ListViewState) -> StoredListView {
    StoredListView {
        sort_field: s.sort.field.token().to_string(),
        sort_dir: s.sort.direction.token().to_string(),
        filters: s.filters.iter().map(|f| f.token().to_string()).collect(),
    }
}

#[cfg(target_arch = "wasm32")]
fn read_url_state() -> Option<ListViewState> {
    // Consume the query captured at startup, not the live (router-stripped) one.
    // Path-matched so a deep link's query only seeds the list it was for; one-shot
    // so later in-app navigations (clean URLs) fall through to sticky / defaults.
    let current_path = web_sys::window()?.location().pathname().ok()?;
    INITIAL_QUERY.with(|c| {
        let mut slot = c.borrow_mut();
        let parsed = match slot.as_ref() {
            Some((path, qs)) if *path == current_path && !qs.is_empty() => {
                ListViewState::from_query(qs)
            }
            // Not our path (or no query): leave the snapshot for the list it belongs
            // to, and fall through to sticky/defaults here.
            _ => return None,
        };
        *slot = None; // consumed — applies once
        parsed
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn read_url_state() -> Option<ListViewState> {
    None
}

#[cfg(target_arch = "wasm32")]
fn write_url_state(state: &ListViewState) {
    use wasm_bindgen::JsValue;
    let Some(win) = web_sys::window() else { return };
    let Ok(history) = win.history() else { return };
    let path = win.location().pathname().unwrap_or_default();
    let q = state.to_query();
    let url = if q.is_empty() {
        path
    } else {
        format!("{path}?{q}")
    };
    // replaceState (not push): filter tweaks mustn't pile up in history, and the
    // path is unchanged so the Dioxus router needn't know.
    let _ = history.replace_state_with_url(&JsValue::NULL, "", Some(&url));
}

#[cfg(not(target_arch = "wasm32"))]
fn write_url_state(_state: &ListViewState) {}

/// Create the `sort` / `filter` signals for a list, seeded from remembered state, and keep both mirrors in sync as they
/// change. `list_key` is the list *type* (`"latest"`, `"podcast"`, `"queue"`, …), shared across ids, so e.g. a
/// podcast-episode sort preference applies to every podcast. `base_filter` carries page-supplied scoping (`podcast_id`,
/// `published_after`) that is preserved and merged onto whatever is restored.
pub fn use_list_view_state(
    list_key: &'static str,
    default_sort: SortSpec,
    base_filter: FilterSpec,
) -> (Signal<SortSpec>, Signal<FilterSpec>) {
    let views = use_context::<Signal<ListViews>>();

    // Seed once. URL (shareable) wins; else sticky views; else page defaults.
    let seed = use_hook(|| {
        read_url_state().or_else(|| views.peek().get(list_key).and_then(stored_to_state))
    });

    let seeded_sort = seed.as_ref().map(|s| s.sort.clone());
    let seeded = seed.clone();

    // Route-supplied scope (podcast_id / published_after). Captured before
    // `base_filter` is moved into the seed below so the scope-sync effect can
    // re-apply it reactively (both fields are `Copy`).
    let scope_podcast_id = base_filter.podcast_id;
    let scope_published_after = base_filter.published_after;

    let sort = use_signal(move || seeded_sort.unwrap_or(default_sort));
    let mut filter = use_signal(move || {
        let mut f = base_filter;
        if let Some(s) = &seeded {
            f.filters = s.filters.clone();
            f.search = s.search.clone();
        }
        f
    });

    // Re-apply the route scope when it changes. On a same-variant, param-only nav (PodcastDetail A → B) Dioxus reuses
    // this component instance, so the one-shot seed above leaves `filter`'s scope pinned to the first route. Sync the
    // scope ids, preserving the user's filters/search, but only when they differ, so the write-back effect below
    // converges instead of looping.
    use_effect(use_reactive!(|scope_podcast_id, scope_published_after| {
        let stale = {
            let f = filter.peek();
            f.podcast_id != scope_podcast_id || f.published_after != scope_published_after
        };
        if stale {
            filter.with_mut(|f| {
                f.podcast_id = scope_podcast_id;
                f.published_after = scope_published_after;
            });
        }
    }));

    // Write-back. Subscribes to sort + filter only; never writes them, so it can't
    // re-trigger the (one-shot) seed or loop.
    let mut initialized = use_signal(|| false);
    let save_gen = use_latest_wins();
    use_effect(move || {
        let state = ListViewState {
            sort: sort.read().clone(),
            filters: filter.read().filters.clone(),
            search: filter.read().search.clone(),
        };
        // peek, not read: bookkeeping signals must not subscribe this effect.
        let first = !*initialized.peek();
        if first {
            initialized.set(true);
        }
        write_url_state(&state);
        if first {
            // First run is the seeded/default state — nothing new to persist.
            return;
        }
        let stored = state_to_stored(&state);
        let generation = save_gen.claim();
        let key = list_key.to_string();
        spawn(async move {
            sleep_ms(SAVE_DEBOUNCE_MS).await;
            // A newer change superseded this one → let it do the write.
            if !save_gen.is_current(generation) {
                return;
            }
            // Write the dedicated views signal (no reactive readers → re-renders
            // nobody) and persist its own store — never the `config` signal.
            let mut views = views;
            views.write().insert(key, stored);
            let snapshot = views.peek().clone();
            ListViewStore::save(&snapshot).await;
        });
    });

    (sort, filter)
}
