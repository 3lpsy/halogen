//! Shared paged episode lists, row state, controls, swipe/bulk actions, and context menus. Use path strings for
//! navigation to avoid depending on the higher-level Route enum.
#![allow(clippy::module_inception, clippy::too_many_arguments)]

mod episode_list;
use halogen_webui_episode_actions::episode_menu;

pub use episode_list::*;
pub use episode_menu::{EpisodeMenuArgs, episode_menu_sections};
