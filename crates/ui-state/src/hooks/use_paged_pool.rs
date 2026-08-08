//! Offline-first paged-list engine, shared by the list pages that render a
//! window over a cached pool and revalidate it page-by-page from the server
//! (Podcasts, Playlists, and the lazy playlist multiselects).
//!
//! The five signals, the IntersectionObserver infinite-scroll `use_future`, and
//! the revalidate `use_effect` skeleton (loading flag + error reset + offline skip)
//! live here once, shared by every list page; a page supplies only its
//! [`Revalidate`] plan — the page-specific params/endpoint/cache call — via the
//! `plan` closure.

use std::future::Future;
use std::pin::Pin;

use dioxus::prelude::*;
use halogen_api::{ApiClient, ApiError};

use super::use_dom_stream::use_dom_stream;
use super::use_latest_wins::use_latest_wins;
use halogen_ui_config::{ClientConfig, api_client_from};

/// Render/fetch window size (one page). Matches the episode lists.
pub const PAGE_SIZE: i32 = 30;

/// How long a just-revalidated server page stays "fresh" before this engine will
/// re-hit the network for it again. Returning to a list / restoring its scroll
/// re-runs the revalidate effect with the same page (stale-while-revalidate), which
/// was re-fetching the identical page every navigation — wasted bytes AND a re-render
/// per landed page. Within this window we skip that repeat fetch and serve the
/// cached pool; the worker's own periodic pull (60s) + WS events still refresh the
/// data underneath, and pull-to-refresh always bypasses this. Well under the worker
/// interval so the list never goes more than a tick stale.
const REVALIDATE_TTL_MS: u64 = 10_000;

/// Whether the revalidate effect should hit the network this run. Pure so the
/// freshness rule is unit-testable. Forces a fetch when pull-to-refresh bumped the
/// nonce (`forced`) or the target page changed; otherwise skips a repeat of a page
/// last fetched within [`REVALIDATE_TTL_MS`]. Crucially the caller leaves `has_more`
/// untouched when this returns `false`, so a skip can't stall paging.
fn should_revalidate(
    forced: bool,
    server_page: i32,
    last: Option<(i32, u64)>,
    now_ms: u64,
) -> bool {
    if forced {
        return true;
    }
    match last {
        Some((p, t)) if p == server_page => now_ms.saturating_sub(t) >= REVALIDATE_TTL_MS,
        _ => true,
    }
}

/// Shared JS for a list's infinite-scroll trigger (the Podcasts/Playlists pools
/// here, and the episode lists). An `IntersectionObserver` tracks whether the
/// bottom sentinel is in view; a persistent 250ms interval re-emits `true` while it
/// stays visible — one observer hit isn't enough, because a server page is async,
/// so the trigger must keep firing until enough new rows push the sentinel off
/// screen. Both the observer and the interval are wired to the caller's
/// `AbortController`, so [`use_dom_stream`] tears them down on unmount.
pub fn infinite_scroll_body(scroll_id: &str, sentinel_id: &str) -> String {
    format!(
        r#"
        const root = document.getElementById('{scroll_id}');
        const sentinel = document.getElementById('{sentinel_id}');
        if (sentinel) {{
            let intersecting = false;
            const obs = new IntersectionObserver((entries) => {{
                for (const e of entries) {{ intersecting = e.isIntersecting; }}
            }}, {{ root: root, rootMargin: '300px' }});
            obs.observe(sentinel);
            signal.addEventListener('abort', () => obs.disconnect());
            const iv = setInterval(() => {{ if (intersecting) {{ dioxus.send(true); }} }}, 250);
            signal.addEventListener('abort', () => clearInterval(iv));
        }}
        "#
    )
}

/// The reactive state of a paged pool. `Copy` (all fields are `Signal`s), so
/// pages can both destructure it for rendering and keep the handle to call
/// [`PagedPool::reset`] / [`PagedPool::refresh`].
#[derive(Clone, Copy)]
pub struct PagedPool {
    /// Server-fetch cursor (page index).
    pub page: Signal<i32>,
    /// Render window size over the cached pool.
    pub visible: Signal<usize>,
    /// Whether the last fetched page was full (more pages may exist).
    pub has_more: Signal<bool>,
    /// A server fetch is in flight.
    pub loading: Signal<bool>,
    /// Last fetch error to surface (only set when the cached pool is empty).
    pub error: Signal<Option<String>>,
    /// Bumped to force a revalidate even when the cursor is already at 0
    /// (pull-to-refresh).
    pub refresh_nonce: Signal<u32>,
    /// `false` until the FIRST revalidate decision has resolved: set `true`
    /// immediately when the first pass serves the cache only (offline / no client /
    /// fresh-within-TTL), and only after the first real server `Fetch` completes.
    /// Lets a consumer whose cache has NOTHING to paint hold its skeletons (and
    /// suppress the empty state) until the cold-load fetch lands, WITHOUT delaying
    /// the offline or warm-navigation paths, which never fetch. A non-empty cache
    /// should paint immediately (stale-while-revalidate) rather than wait on this.
    pub initial_settled: Signal<bool>,
}

impl PagedPool {
    /// Reset the window + cursor to the top — a sort/filter/search change starts
    /// a fresh (re-ordered / filtered) page sequence under server-side ordering.
    ///
    /// Also bumps the nonce to FORCE the page-0 fetch: the freshness TTL keys on
    /// the page number alone, and a predicate change usually lands with the
    /// cursor already at 0 and "fresh" — without the force, the fetch for the
    /// NEW predicate was silently TTL-skipped and the list showed the old
    /// predicate's cached rows until the worker's next periodic pull. (Every
    /// caller of `reset` is a predicate change; warm navigation/scroll-restore
    /// never calls it, so the TTL still serves its purpose there.)
    pub fn reset(self) {
        let PagedPool {
            mut page,
            mut visible,
            mut has_more,
            mut refresh_nonce,
            ..
        } = self;
        page.set(0);
        has_more.set(true);
        visible.set(PAGE_SIZE as usize);
        refresh_nonce += 1;
    }

    /// Reset to the top and force a re-fetch even if the cursor was already 0
    /// (pull-to-refresh). Alias of [`Self::reset`] now that reset itself forces
    /// — kept for call-site intent ("refresh the data" vs "new predicate").
    pub fn refresh(self) {
        self.reset();
    }
}

/// Outcome of a single page fetch, reported back to the pool.
pub struct FetchResult {
    /// Whether the fetched page was full (`len == PAGE_SIZE`).
    pub has_more: bool,
    /// An error to surface, or `None` to stay silent (e.g. the cached pool can
    /// still render, so a transient fetch failure is swallowed).
    pub error: Option<String>,
}

/// What a revalidate pass should do for the requested page.
pub enum Revalidate {
    /// Render the cached pool only — no server fetch (offline, or no client).
    /// Stops the infinite-scroll cursor.
    Skip,
    /// Run this future: it fetches the page, caches it into the pool, and reports
    /// [`FetchResult`]. The pool owns the surrounding loading/error bookkeeping.
    Fetch(Pin<Box<dyn Future<Output = FetchResult>>>),
}

/// Build the standard offline-first [`Revalidate`] plan shared by the collection
/// list pages (Podcasts, Playlists). It owns the boilerplate every such page
/// repeated verbatim: honor "Go Offline" (`manual_offline`) and a missing client
/// with [`Revalidate::Skip`], then on a real fetch fold the result into a
/// [`FetchResult`] — `has_more` from a full page, and an error surfaced **only**
/// when the cached pool is empty (`is_empty`), so a transient failure with rows
/// already cached stays silent.
///
/// What stays per-page is what differs: `make_request` builds the page's params and
/// calls the endpoint — it reads any reactive sort/search **synchronously** (before
/// the future it returns), so the pool re-fetches when they change, exactly as a
/// hand-rolled plan did; `cache` hands the fetched rows to the worker; `is_empty`
/// reports whether the cached pool is currently empty.
pub fn paged_list_plan<T, MkFut, Fut>(
    config: Signal<ClientConfig>,
    is_empty: impl Fn() -> bool + Copy + 'static,
    cache: impl Fn(Vec<T>) + Copy + 'static,
    make_request: MkFut,
) -> impl Fn(i32) -> Revalidate + 'static
where
    T: 'static,
    MkFut: Fn(ApiClient, i32) -> Fut + 'static,
    Fut: Future<Output = Result<Vec<T>, ApiError>> + 'static,
{
    // Subscribe the revalidate effect ONLY to the auth-relevant config slice (offline
    // toggle + server URL + token), via a PartialEq memo. Reading the whole
    // `Signal<ClientConfig>` directly would re-run the effect — and re-fetch every
    // mounted paged list — on ANY config edit (font size, playback rate from the
    // always-mounted mini-player, nav order, …). The memo gates re-runs to real auth
    // changes, which legitimately should revalidate (reconnect, token refresh, setup).
    let auth = use_memo(move || {
        let c = config.read();
        (
            c.manual_offline,
            c.server_url.clone(),
            c.access_token.clone(),
        )
    });
    move |server_page| {
        let (manual_offline, server_url, token) = auth();
        if manual_offline {
            // "Go Offline": render the cached pool only — no server fetch.
            return Revalidate::Skip;
        }
        let Some(client) = api_client_from(server_url.as_deref(), token.as_deref()) else {
            return Revalidate::Skip;
        };
        // Called synchronously inside the revalidate effect — any sort/search reads
        // here subscribe it, so the fetch re-runs when they change.
        let fut = make_request(client, server_page);
        Revalidate::Fetch(Box::pin(async move {
            match fut.await {
                Ok(data) => {
                    let has_more = data.len() as i32 == PAGE_SIZE;
                    cache(data);
                    FetchResult {
                        has_more,
                        error: None,
                    }
                }
                // The cached pool may still render; only surface an error if it's
                // empty (genuinely nothing to show).
                Err(e) => FetchResult {
                    has_more: false,
                    error: is_empty().then(|| e.to_string()),
                },
            }
        }))
    }
}

/// Drive an offline-first paged pool with the built-in infinite-scroll observer.
///
/// `scroll_id` / `sentinel_id` are the dom ids of the scroll container and the
/// bottom sentinel (the page must render both). See [`use_paged_pool_with`] for
/// the variant that lets a consumer drive its own scroll.
/// `cached_len` reports the current size of the cached pool the page renders a
/// window over — it bounds window growth so a list shorter than the viewport can't
/// grow `visible` forever (see [`use_paged_pool_with`]).
pub fn use_paged_pool(
    scroll_id: &str,
    sentinel_id: &str,
    cached_len: impl Fn() -> usize + 'static,
    plan: impl Fn(i32) -> Revalidate + 'static,
) -> PagedPool {
    use_paged_pool_with(Some((scroll_id, sentinel_id)), cached_len, plan)
}

/// [`use_paged_pool`] with optional built-in scroll management.
///
/// `plan(page)` is called inside the revalidate effect: it reads the page's
/// reactive inputs (sort/search/config) **synchronously** — so the effect
/// re-fetches when any of them change — and returns the page-specific
/// [`Revalidate`] action. The effect always subscribes to `page` (cursor) and
/// `refresh_nonce` (pull-to-refresh).
///
/// When `scroll` is `Some((scroll_id, sentinel_id))` the hook installs the shared
/// IntersectionObserver infinite-scroll (grow `visible`, advance `page` when
/// `has_more`). Pass `None` when the consumer manages its own scroll (e.g. a
/// multi-mode list that must gate window growth on a local count) — it then drives
/// `page`/`visible` itself off the returned [`PagedPool`] (and `cached_len` is
/// unused, since the built-in handler never fires).
///
/// `cached_len()` reports the current cached-pool size; the built-in handler grows
/// `visible` only while there's more to reveal (`visible < cached_len`) or another
/// server page to fetch (`has_more`). Without it, a list shorter than the viewport
/// keeps the bottom sentinel on-screen and the persistent 250ms trigger would grow
/// `visible` unboundedly — a permanent 4×/sec re-render of the whole list.
pub fn use_paged_pool_with(
    scroll: Option<(&str, &str)>,
    cached_len: impl Fn() -> usize + 'static,
    plan: impl Fn(i32) -> Revalidate + 'static,
) -> PagedPool {
    let mut page = use_signal::<i32>(|| 0);
    let mut visible = use_signal(|| PAGE_SIZE as usize);
    let mut has_more = use_signal(|| true);
    let mut loading = use_signal(|| false);
    let mut error = use_signal::<Option<String>>(|| None);
    let refresh_nonce = use_signal(|| 0u32);
    let revalidate_gen = use_latest_wins();
    // Freshness bookkeeping for the revalidate effect (peeked, never subscribed, so
    // stamping can't re-trigger the effect): the last server page successfully
    // revalidated + when (epoch-ms), and the last pull-to-refresh nonce seen so a
    // nonce bump forces a fetch through the TTL. See [`should_revalidate`].
    let mut last_revalidate = use_signal(|| None::<(i32, u64)>);
    let mut last_nonce = use_signal(|| 0u32);
    // First-paint gate for consumers (see `PagedPool::initial_settled`). Starts
    // `false`; the revalidate effect flips it `true` once the first decision has
    // resolved (a cache-only serve flips it at once; a real fetch flips it when it
    // lands). Idempotent — only the first flip matters.
    let mut initial_settled = use_signal(|| false);

    // Revalidate the current server page into the pool. The worker upserts the
    // fetched rows and publishes, re-rendering the page from the pool — so this
    // never subscribes to EpisodeState and the dispatch can't loop it.
    use_effect(move || {
        let nonce = refresh_nonce(); // subscribe: pull-to-refresh forces a revalidate
        let server_page = page();
        // A nonce bump (pull-to-refresh / `pool.refresh()`) always forces a fetch;
        // a plain re-mount / scroll-restore with the same page inside the freshness
        // window is skipped (serve the cached pool). On a skip we DON'T touch
        // `has_more`, so paging state from the last real fetch is preserved.
        let forced = nonce != *last_nonce.peek();
        if forced {
            last_nonce.set(nonce);
        }
        if !should_revalidate(
            forced,
            server_page,
            *last_revalidate.peek(),
            halogen_ui_platform::time::now_ms(),
        ) {
            // Fresh within TTL (warm nav / scroll-restore): no fetch, so the cache
            // is authoritative now — release any first-paint gate immediately.
            if !*initial_settled.peek() {
                initial_settled.set(true);
            }
            return;
        }
        match plan(server_page) {
            // Offline / no client: render the cached pool only, stop the cursor.
            Revalidate::Skip => {
                has_more.set(false);
                // No fetch will land — the cache is all we have, so paint it now.
                if !*initial_settled.peek() {
                    initial_settled.set(true);
                }
            }
            Revalidate::Fetch(fut) => {
                // Token this fetch so a stale in-flight one can't clobber a newer
                // revalidate's bookkeeping (mirrors `save_gen` in
                // `use_list_view_state`). Bumped synchronously here; captured before
                // the await, re-checked after.
                let generation = revalidate_gen.claim();
                spawn(async move {
                    loading.set(true);
                    error.set(None);
                    let result = fut.await;
                    // A newer revalidate started while we awaited → let it own the
                    // bookkeeping; don't write a stale result.
                    if !revalidate_gen.is_current(generation) {
                        return;
                    }
                    let clean = result.error.is_none();
                    has_more.set(result.has_more);
                    error.set(result.error);
                    loading.set(false);
                    // The first real fetch has landed (rows dispatched to the worker):
                    // release the first-paint gate so a gated consumer can now paint
                    // the freshly-fetched page. A consumer that gated absorbs the
                    // fetch→upsert→publish lag with its own settle (last-wins).
                    if !*initial_settled.peek() {
                        initial_settled.set(true);
                    }
                    // Stamp the freshness clock only on a clean fetch, so an errored
                    // page retries immediately on the next mount instead of waiting.
                    if clean {
                        last_revalidate
                            .set(Some((server_page, halogen_ui_platform::time::now_ms())));
                    }
                });
            }
        }
    });

    // Infinite scroll: grow the render window and advance the server cursor when
    // the sentinel scrolls into view (a persistent 250ms trigger). Own the ids so
    // the build closure is `'static`. The shared observer/interval JS is torn down
    // on unmount by `use_dom_stream` (the bare `setInterval`/observer it registers
    // would otherwise leak per list nav). Skipped when the consumer manages scroll.
    // Call `use_dom_stream` unconditionally (it's a hook): a `None` `scroll` builds
    // an empty body, which the harness treats as "nothing to register" (no eval, no
    // messages) — same no-op idiom as `use_scroll_memory`. This keeps the hook count
    // stable even if a caller flips `Some`↔`None` across renders.
    let scroll = scroll.map(|(s, t)| (s.to_string(), t.to_string()));
    use_dom_stream(
        move || match &scroll {
            Some((scroll_id, sentinel_id)) => infinite_scroll_body(scroll_id, sentinel_id),
            None => String::new(),
        },
        move |_: bool| {
            // Grow the window / advance the cursor only when there's actually more to
            // reveal — more cached rows to window in (`visible < cached_len`) or
            // another server page to fetch (`has_more`). Without the `cached_len`
            // gate, a list shorter than the viewport keeps the sentinel on-screen and
            // the 250ms trigger would grow `visible` forever (a 4Hz re-render).
            let more = has_more() || visible() < cached_len();
            if more && !loading() {
                visible += PAGE_SIZE as usize;
                if has_more() {
                    page += 1;
                }
            }
        },
    );

    PagedPool {
        page,
        visible,
        has_more,
        loading,
        error,
        refresh_nonce,
        initial_settled,
    }
}

#[cfg(test)]
mod revalidate_freshness_tests {
    use super::{REVALIDATE_TTL_MS, should_revalidate};

    #[test]
    fn first_ever_visit_fetches() {
        // No prior fetch recorded → always revalidate.
        assert!(should_revalidate(false, 0, None, 10_000));
    }

    #[test]
    fn pull_to_refresh_always_fetches_even_when_fresh() {
        // forced (nonce bump) bypasses the TTL for the same page.
        assert!(should_revalidate(true, 3, Some((3, 9_500)), 10_000));
    }

    #[test]
    fn same_page_within_window_is_skipped_boundary_exclusive() {
        // Just inside the window → skip; exactly at / past the edge → fetch.
        assert!(!should_revalidate(
            false,
            3,
            Some((3, 1_000)),
            1_000 + REVALIDATE_TTL_MS - 1
        ));
        assert!(!should_revalidate(false, 3, Some((3, 1_000)), 1_000)); // same instant
        assert!(should_revalidate(
            false,
            3,
            Some((3, 1_000)),
            1_000 + REVALIDATE_TTL_MS
        ));
    }

    #[test]
    fn a_different_page_always_fetches() {
        // Scrolling to the next page (or any page change) is never TTL-skipped.
        assert!(should_revalidate(false, 4, Some((3, 9_999)), 10_000));
    }

    #[test]
    fn backwards_clock_does_not_wedge_a_fetch_off() {
        // now < last: saturating_sub → 0 < TTL would be "skip", which is harmless
        // (one extra cache-serve); a forward jump past the window re-fetches.
        assert!(!should_revalidate(false, 3, Some((3, 10_000)), 1_000));
    }
}
