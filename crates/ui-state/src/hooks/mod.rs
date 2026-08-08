//! Typed access hooks — the single way pages/components reach shared state and
//! the worker. Replaces ad-hoc `use_context::<…>()` scattered through the UI.

mod context;
mod use_confirm_action;
mod use_deep_link_fetch;
mod use_dom_stream;
mod use_episodes;
mod use_form;
mod use_is_admin;
mod use_latest_wins;
mod use_list_view_state;
mod use_paged_pool;
mod use_pull_to_refresh;
mod use_row_memory;
mod use_scroll_memory;
#[cfg(target_arch = "wasm32")]
mod use_window_event;

pub use context::{
    ToastLevel, use_accounts, use_config, use_connection, use_discover_store, use_dispatch,
    use_downloads, use_history, use_now_playing, use_now_playing_identity, use_play_context,
    use_playbacks, use_player_controller, use_playlists, use_podcasts, use_session, use_store,
    use_toast, use_toasts,
};
pub use use_confirm_action::use_confirm_action;
pub use use_deep_link_fetch::{deep_link_placeholder, use_deep_link_resource};
pub use use_dom_stream::use_dom_stream;
pub use use_episodes::{use_connection_health, use_episodes, use_is_offline, use_sync_status};
pub use use_form::{FormState, use_form_state};
pub use use_is_admin::use_is_admin;
pub use use_latest_wins::{LatestWins, use_latest_wins};
pub use use_list_view_state::{
    capture_initial_query, read_initial_query_param, use_list_view_state,
};
pub use use_paged_pool::{
    FetchResult, PAGE_SIZE, PagedPool, Revalidate, infinite_scroll_body, paged_list_plan,
    use_paged_pool, use_paged_pool_with,
};
pub use use_pull_to_refresh::{PULL_THRESHOLD, PullPhase, use_pull_to_refresh};
pub use use_row_memory::{ListRowsSnapshot, RowMemory, use_row_memory};
pub use use_scroll_memory::{
    ScrollMemory, ScrollPaging, list_stamp, use_paged_scroll_memory, use_scroll_memory,
};
#[cfg(target_arch = "wasm32")]
pub use use_window_event::use_window_event;
