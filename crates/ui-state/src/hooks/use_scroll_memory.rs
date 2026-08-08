//! In-memory scroll-position recovery for lazy/paginated lists.
//!
//! Returning to a list (`/latest`, `/queue`, `/podcasts`, `/playlists`,
//! `/downloads`) should land the user back where they were. The lists are lazy:
//! on remount they render only the first window (`visible = PAGE_SIZE`), so the
//! scroll container isn't tall enough to hold the old offset. So we remember BOTH
//! the render window (`visible`/`page`/`has_more`) and the `scroll_top`, restore
//! the window first (it repaints all prior rows from the persisted pool in one
//! pass — see `episode_list/paged.rs`), then re-apply `scroll_top` once the rows
//! have painted.
//!
//! Lifetime is the RUNNING SESSION ONLY — backed by a `Signal<ScrollMemory>`
//! provided in `AppLayout`, never persisted to disk (unlike `ListViews`, which
//! this mirrors structurally). A fresh app launch starts at the top.
//!
//! The store has NO reactive readers (seed `peek`s; saves `write` only), so writes
//! re-render nobody — same discipline as [`use_list_view_state`].

use std::collections::HashMap;

use dioxus::prelude::*;

use super::use_dom_stream::use_dom_stream;
use super::use_paged_pool::PagedPool;
use halogen_ui_listview::{FilterSpec, SortSpec};

/// Remembered window + scroll offset for one list, keyed by list type.
#[derive(Clone, Copy, Debug, Default)]
pub struct ListScrollState {
    /// Render-window size to restore so prior rows repaint from the pool.
    pub visible: usize,
    /// Server-page cursor (relevant for cache-query / server-paged lists; unused
    /// by id-list lists, where it stays 0).
    pub page: i32,
    /// Advisory: seeded, but the list's revalidate effect corrects it on next fetch.
    pub has_more: bool,
    /// The scroll offset to restore once enough rows have painted.
    pub scroll_top: f64,
    /// Hash of the sort+filter the offset was recorded against. On restore we only
    /// re-apply `scroll_top` when this still matches — a re-sorted/filtered list
    /// makes a pixel offset meaningless (we still restore the window).
    pub stamp: u64,
}

/// In-memory map keyed by list type (`"latest"`, `"queue"`, …) — same key space
/// as [`use_list_view_state`]. Provided as a `Signal<ScrollMemory>` in `AppLayout`.
pub type ScrollMemory = HashMap<String, ListScrollState>;

/// The list's own window/cursor signals, handed to [`use_scroll_memory`] so it can
/// seed them at mount and mirror them on change.
#[derive(Clone, Copy)]
pub struct ScrollPaging {
    pub visible: Signal<usize>,
    pub page: Signal<i32>,
    pub has_more: Signal<bool>,
}

/// A cheap, stable hash of a list's sort + filter — the validity stamp for a saved
/// scroll offset. Shared by every list so the stamp space is consistent.
pub fn list_stamp(sort: &SortSpec, filter: &FilterSpec) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    sort.field.token().hash(&mut h);
    sort.direction.token().hash(&mut h);
    filter.search.hash(&mut h);
    filter.podcast_id.hash(&mut h);
    // `published_after` is a date type without Hash — its debug form is stable enough.
    format!("{:?}", filter.published_after).hash(&mut h);
    for f in &filter.filters {
        f.token().hash(&mut h);
    }
    h.finish()
}

/// JS for a list's scroll recorder, with an optional pre-restore — one script so
/// the restore is guaranteed to run *before* the recorder arms.
///
/// If `restore_target` is set, RAF-polls until the lazily-rendered window is tall
/// enough to hold the offset (or 60 frames pass), jumps to it (clamped), and only
/// THEN arms the recorder — restoring before recording avoids persisting the
/// pre-restore top/0. The recorder reports `scrollTop` on a slow heartbeat and on
/// teardown (`pagehide` / tab hidden), reading the value directly rather than
/// reacting to every `scroll` event (which would record a transient mount-time 0).
///
/// Every timer/listener is wired to the caller's `AbortController` (`signal`), so
/// [`use_dom_stream`] tears them all down on unmount — the `setInterval` +
/// window/document listeners don't leak across list navs.
fn scroll_record_body(container_id: &str, restore_target: Option<f64>) -> String {
    let restore = match restore_target {
        Some(top) => format!(
            r#"
            let __tries = 0;
            const __restore = () => {{
                const max = root.scrollHeight - root.clientHeight;
                if (max >= {top} || __tries > 60) {{
                    root.scrollTop = Math.min({top}, Math.max(0, max));
                    arm();
                    return;
                }}
                __tries++;
                requestAnimationFrame(__restore);
            }};
            requestAnimationFrame(__restore);
            "#
        ),
        None => "arm();".to_string(),
    };
    format!(
        r#"
        const root = document.getElementById('{container_id}');
        if (root) {{
            const arm = () => {{
                let last = -1;
                const report = () => {{ const t = root.scrollTop; if (t !== last) {{ last = t; dioxus.send(t); }} }};
                const iv = setInterval(report, 500);
                signal.addEventListener('abort', () => clearInterval(iv));
                window.addEventListener('pagehide', () => dioxus.send(root.scrollTop), {{ signal }});
                document.addEventListener('visibilitychange', () => {{
                    if (document.visibilityState === 'hidden') dioxus.send(root.scrollTop);
                }}, {{ signal }});
            }};
            {restore}
        }}
        "#
    )
}

/// Restore + record scroll position for a list. Call ONCE per list mount.
///
/// - `key`: list type (`"latest"`, …); `None` makes the whole hook a no-op (so the
///   shared `EpisodeList` can call it unconditionally on every consumer).
/// - `container_id`: the INNER scroll element id (`"episode-scroll"`, …).
/// - `paging`: the list's window/cursor signals, seeded here at mount.
/// - `stamp`: a memo of the current sort+filter (see [`list_stamp`]).
pub fn use_scroll_memory(
    key: Option<&'static str>,
    container_id: &'static str,
    paging: ScrollPaging,
    stamp: Memo<u64>,
) {
    let ScrollPaging {
        mut visible,
        mut page,
        mut has_more,
    } = paging;
    // Optional so a standalone-mounted list (unit tests) degrades to no-op.
    let store = try_consume_context::<Signal<ScrollMemory>>();

    // Seed once, synchronously during the first build — before the list's load
    // effects read `visible()` — so they query the restored window on pass one (no
    // first-render flash). Snapshot the saved state here, before any async write.
    let snapshot: Option<ListScrollState> = use_hook(|| {
        let (Some(key), Some(store)) = (key, store) else {
            return None;
        };
        let saved = store.peek().get(key).copied()?;
        visible.set(saved.visible.max(1));
        page.set(saved.page);
        has_more.set(saved.has_more);
        Some(saved)
    });

    // Mirror the window/cursor into the store whenever it changes (non-reactive
    // write → re-renders nobody). Subscribes to visible/page/has_more. The `stamp`
    // is owned by the recorder below, which pairs it with the offset it was actually
    // scrolled under — writing it here too would stamp the *current* view onto a
    // stale offset on a re-sort (no scroll), wrongly revalidating it. The window
    // restores regardless of sort, so it needs no stamp.
    use_effect(move || {
        let (Some(key), Some(mut store)) = (key, store) else {
            return;
        };
        let (v, p, m) = (visible(), page(), has_more());
        let mut map = store.write();
        let entry = map.entry(key.to_string()).or_default();
        entry.visible = v;
        entry.page = p;
        entry.has_more = m;
    });

    // Armed restore → then record, in one self-cleaning script. The recorder only
    // arms after the restore jumps to the saved offset, so it never observes (and
    // persists) the pre-restore top/0. The restore offset is only valid if the
    // sort/filter still matches what it was saved under (a pixel offset is
    // meaningless on a re-sorted list — the window restores regardless, above).
    let restore_target = snapshot.and_then(|snap| {
        (snap.scroll_top > 0.0 && snap.stamp == *stamp.peek()).then_some(snap.scroll_top)
    });
    let recording = key.is_some() && store.is_some();
    use_dom_stream(
        move || {
            if recording {
                scroll_record_body(container_id, restore_target)
            } else {
                String::new()
            }
        },
        move |top: f64| {
            let (Some(key), Some(mut store)) = (key, store) else {
                return;
            };
            let st = *stamp.peek();
            let mut map = store.write();
            let entry = map.entry(key.to_string()).or_default();
            entry.scroll_top = top;
            entry.stamp = st;
        },
    );
}

/// Register a paged pool with scroll-memory, stamped by the list's sort + filter (so
/// a restored offset invalidates across sort/search changes). The shared scroll
/// wiring the offline-first list pages (Podcasts, Playlists) otherwise repeat
/// verbatim: the [`list_stamp`] memo plus the [`use_scroll_memory`] call over the
/// pool's window signals. `key` is the list type (`"podcasts"`); `container_id` is
/// the inner scroll element id (`"podcast-scroll"`).
pub fn use_paged_scroll_memory(
    key: &'static str,
    container_id: &'static str,
    pool: PagedPool,
    sort: Signal<SortSpec>,
    filter: Signal<FilterSpec>,
) {
    let PagedPool {
        page,
        visible,
        has_more,
        ..
    } = pool;
    let stamp = use_memo(move || list_stamp(&sort.read(), &filter.read()));
    use_scroll_memory(
        Some(key),
        container_id,
        ScrollPaging {
            visible,
            page,
            has_more,
        },
        stamp,
    );
}
