#![allow(clippy::module_inception, clippy::too_many_arguments)]

//! `halogen-ui-episode-list` — the self-contained episode-list cluster (the paged
//! list, per-row component, sort/filter/search controls, swipe + bulk actions) and
//! its fused per-row context menu (`episode_menu`). The largest single view
//! component group: a self-contained component group that compiles in parallel
//! with the views layer.
//!
//! Route-free: its handful of navigations use path strings (`navigator().push(
//! "/episodes/{id}")`) instead of the `Route` enum, which lives up in `ui-views`.

mod episode_list;
mod episode_menu;

pub use episode_list::*;
pub use episode_menu::{EpisodeMenuArgs, episode_menu_sections};
