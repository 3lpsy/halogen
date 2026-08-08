mod action;
mod bulk_menu;
mod controls;
mod item;
mod list;
mod paged;
mod query;
mod row_parts;
mod row_state;

pub use controls::ListControls;
pub use item::{CloudProgressOrSpinner, EpisodeListItem, ProgressOrSpinner};
pub use list::EpisodeList;
// `EpisodeRowState` + its builder/menu-context are the shared episode row-state
// surface the detail page (outside this module) also reads through; the swipe
// dispatcher and the row sub-components stay internal to the cluster.
pub use row_state::{EpisodeRowState, MenuListContext, resolve_episode_row_state};
// The sort/filter/swipe view-data types live in `halogen-ui-listview` (shared with
// services + hooks). Re-export them here so the existing `halogen_ui_widgets::*`
// facade (via `components::mod`'s `pub use episode_list::*`) keeps working.
pub use halogen_ui_listview::{
    EpisodeFilter, FilterSpec, ItemVariant, ListSource, ListViewState, OrderDirection, SortField,
    SortSpec, SwipeAction, SwipeConfig, search_sort,
};
