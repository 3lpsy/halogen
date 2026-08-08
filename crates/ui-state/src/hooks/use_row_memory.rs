//! Last-committed-rows memory for the list pages: seeds the render signals
//! synchronously at mount so a re-navigation paints the previous rows on frame
//! one (stale-while-revalidate — the list's resolve effects then diff-commit).
//! Session-only, provided in `AppLayout` (recreated on account switch).

use std::collections::HashMap;

use dioxus::prelude::*;

use halogen_wire::EpisodeData;

/// The last committed row set + counters for one list, keyed by list type.
#[derive(Clone, Debug, Default)]
pub struct ListRowsSnapshot {
    /// Rows exactly as last committed (resolved, filtered, sorted, windowed).
    pub rows: Vec<EpisodeData>,
    /// Filtered pool size at commit time (reorder mirror axis / scroll gate).
    pub pool_count: usize,
    /// Unresolved id-list membership size at commit time (scroll gate).
    pub id_total: usize,
    /// Sort+filter+source hash the rows were committed under; restore only on match.
    pub stamp: u64,
}

/// In-memory map keyed by list type — same key space as `use_scroll_memory`.
pub type RowMemory = HashMap<String, ListRowsSnapshot>;

/// Seed + mirror a list's committed rows; call ONCE per list mount.
/// `None` key = no-op. Returns whether a snapshot was restored.
pub fn use_row_memory(
    key: Option<&'static str>,
    stamp: Memo<u64>,
    mut episodes: Signal<Vec<EpisodeData>>,
    mut pool_count: Signal<usize>,
    mut id_total: Signal<usize>,
) -> bool {
    // Optional so a standalone-mounted list (unit tests) degrades to no-op.
    let store = try_consume_context::<Signal<RowMemory>>();

    // Seed synchronously during the first build so rows paint on frame one.
    // Empty snapshots aren't restored — the skeletons cover that case honestly.
    let restored = use_hook(|| {
        let (Some(key), Some(store)) = (key, store) else {
            return false;
        };
        let Some(saved) = store.peek().get(key).cloned() else {
            return false;
        };
        if saved.rows.is_empty() || saved.stamp != *stamp.peek() {
            return false;
        }
        episodes.set(saved.rows);
        pool_count.set(saved.pool_count);
        id_total.set(saved.id_total);
        true
    });

    // Mirror every commit (write-only store: re-renders nobody). Stamp is peeked
    // so a sort change alone can't stamp the new query onto the old rows.
    use_effect(move || {
        let (Some(key), Some(mut store)) = (key, store) else {
            return;
        };
        let snapshot = ListRowsSnapshot {
            rows: episodes.read().clone(),
            pool_count: pool_count(),
            id_total: id_total(),
            stamp: *stamp.peek(),
        };
        store.write().insert(key.to_string(), snapshot);
    });

    restored
}
