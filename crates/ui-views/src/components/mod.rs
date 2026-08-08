// The Route-free reusable widgets moved to `halogen-ui-widgets`; re-export them so
// the `crate::components::*` facade (used throughout pages + smart components) is
// unchanged. The Route-coupled "smart" components stay here (they reference the
// `Route` enum, which lives in this crate).
pub use halogen_ui_widgets::*;
// The episode-list cluster + its row menu live in `halogen-ui-episode-list`;
// re-export so `crate::components::{EpisodeList, EpisodeMenuArgs, …}` is unchanged.
pub use halogen_ui_episode_list::*;

mod dock;
mod nav;
mod navbar;
mod player;
mod playlist_menu;
mod playlist_multiselect;
mod podcast_menu;
mod sidebar;

pub use dock::Dock;
pub use nav::{RouteHistory, nav_icon, nav_items, nav_label};
pub use navbar::Navbar;
pub use player::{MiniPlayer, NowPlayingScreen, UpNext};
pub use playlist_menu::playlist_menu_sections;
pub use playlist_multiselect::{PlaylistMultiselect, PlaylistPickerScaffold};
pub use podcast_menu::podcast_menu_sections;
pub use sidebar::Sidebar;
