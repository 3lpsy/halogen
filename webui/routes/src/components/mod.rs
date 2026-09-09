// The Route-free reusable widgets moved to `halogen-webui-component-widgets`; re-export them so
// the `crate::components::*` facade (used throughout pages + smart components) is
// unchanged. The Route-coupled "smart" components stay here (they reference the
// `Route` enum, which lives in this crate).
pub use halogen_webui_component_widgets::*;
// The episode-list cluster + its row menu live in `halogen-webui-episode-list`;
// re-export so `crate::components::{EpisodeList, EpisodeMenuArgs, …}` is unchanged.
pub use halogen_webui_episode_list::*;

mod dock;
mod nav;
mod navbar;
mod sidebar;

pub use dock::Dock;
pub use nav::{RouteHistory, nav_icon, nav_items, nav_label};
pub use navbar::Navbar;
pub use sidebar::Sidebar;

pub use halogen_webui_library_widgets::*;
pub use halogen_webui_player_controls::*;
