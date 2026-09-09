//! Single-episode pages: the detail view, the read-only metadata view, and the
//! add-to-playlist pickers.
mod components {
    pub use halogen_webui_component_widgets::{
        Artwork, BackButton, ConfirmLinkModal, DetailHeaderBar, KebabButton, RichText, use_confirm,
        use_quick_menu,
    };
    pub use halogen_webui_episode_list::{
        CloudProgressOrSpinner, EpisodeMenuArgs, EpisodeRowState, MenuListContext,
        ProgressOrSpinner, episode_menu_sections, resolve_episode_row_state,
    };
    pub use halogen_webui_library_widgets::PlaylistPickerScaffold;
}

pub mod bulk_episode_playlists;
pub mod episode_detail;
pub mod episode_metadata;
pub mod episode_playlists;
