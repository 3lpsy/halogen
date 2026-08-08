//! History paging cursor — the worker-owned reactive slice.
//!
//! Tiny by design: just the `GET /playbacks` paging cursor (the History list's
//! ordered membership is derived from [`PlaybackState`](crate::PlaybackState), not
//! held here). Its own signal so advancing the cursor as History pages in only
//! re-renders the History page-ahead effect, not every `EpisodeState` subscriber. The
//! sync worker is the only writer; the UI reads it reactively (see
//! `halogen_ui_state::hooks::use_history`).

/// History paging cursor over `GET /playbacks` (updated_at desc). The History list
/// grows the playbacks overlay page-by-page on scroll; these track the next page to
/// request and whether the server has more. Reset to `(true, 0)` on refresh.
///
/// `PartialEq` is load-bearing (same reason as on `EpisodeState`): memo slices over this
/// signal gate re-renders on it. The `historystate_is_partialeq` canary guards it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HistoryState {
    /// Whether the server has more `/playbacks` pages past the loaded cursor.
    pub history_has_more: bool,
    /// The next `/playbacks` page to request.
    pub history_next_page: i32,
}

impl Default for HistoryState {
    /// The cold-start cursor: there may be more pages (`true`) and we start from page 0.
    fn default() -> Self {
        Self {
            history_has_more: true,
            history_next_page: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Compile-time canary for the load-bearing derive (mirrors
    /// `appstate_is_partialeq`): memo slices over this signal need
    /// `HistoryState: PartialEq` to gate re-renders.
    #[test]
    fn historystate_is_partialeq() {
        fn assert_partial_eq<T: PartialEq>() {}
        assert_partial_eq::<HistoryState>();
    }

    #[test]
    fn default_starts_at_page_zero_with_more() {
        let h = HistoryState::default();
        assert!(h.history_has_more);
        assert_eq!(h.history_next_page, 0);
    }
}
