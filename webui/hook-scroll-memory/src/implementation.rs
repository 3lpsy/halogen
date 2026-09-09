//! Remember the paged window and scroll offset for this session. Restore cached rows before applying the offset so the
//! list is tall enough. `ScrollMemory` is not persisted and has no reactive readers, so saves do not rerender
//! components.

use std::collections::HashMap;

use dioxus::prelude::*;

use halogen_webui_hook_dom_stream::use_dom_stream;
use halogen_webui_hook_paged_pool::PagedPool;
use halogen_webui_listview::{FilterSpec, SortSpec};

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

/// Wait up to 60 frames for the restored window, apply its clamped offset, then arm recording to avoid saving
/// mount-time zero. Record on a slow heartbeat, pagehide, and tab hiding; the caller's abort signal cleans up all
/// listeners and timers.
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

/// Restore and record once per mount using the inner `container_id`, paging signals, and sort/filter `stamp`. `key`
/// identifies the list; `None` makes the hook inert while preserving unconditional hook calls.
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

    // Mirror the window/cursor into the store whenever it changes (non-reactive write → re-renders nobody). Subscribes
    // to visible/page/has_more. The `stamp` is owned by the recorder below, which pairs it with the offset it was
    // scrolled under, writing it here too would stamp the *current* view onto a stale offset on a re-sort (no scroll),
    // wrongly revalidating it. The window restores regardless of sort, so it needs no stamp.
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

    // Armed restore → then record, in one self-cleaning script. The recorder only arms after the restore jumps to the
    // saved offset, so it never observes (and persists) the pre-restore top/0. The restore offset is only valid if the
    // sort/filter still matches what it was saved under (a pixel offset is meaningless on a re-sorted list, the window
    // restores regardless, above).
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

/// Register a paged pool with scroll-memory, stamped by the list's sort + filter (so a restored offset invalidates
/// across sort/search changes). The shared scroll wiring the offline-first list pages (Podcasts, Playlists) otherwise
/// repeat verbatim: the [`list_stamp`] memo plus the [`use_scroll_memory`] call over the pool's window signals. `key`
/// is the list type (`"podcasts"`); `container_id` is the inner scroll element id (`"podcast-scroll"`).
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
