//! Reusable forms, dialogs, artwork, rich text, and list controls.
#![allow(clippy::module_inception, clippy::too_many_arguments)]

mod artwork;
mod back_button;
mod confirm;
mod confirm_modal;
mod controls_shared;
mod download_prefs;
mod drag_reorder;
mod expandable_podcast_description;
mod form;
mod form_page;
// Whitelisted rich-text HTML AST (parse/Block/Inline/to_plain), used by RichText
// here and by the episode-list (`to_plain`).
pub mod html;
mod kebab_button;
mod marquee;
mod menu_actions;
mod playback_prefs;
mod pull_to_refresh;
mod quick_context_menu;
mod resource_view;
mod rich_text;
mod sort_search_controls;
mod toast;

// Lower-layer view-data types these widgets reference (the controls/sort/filter set), from `halogen-webui-listview`.
// Crate-internal re-export so intra-crate `crate::SortSpec` paths resolve, without leaking through
// `halogen_webui_component_widgets::*` (ui-views re-exports the `halogen-webui-listview` view-data types from one
// explicit place instead, avoiding a double glob-export).
pub(crate) use halogen_webui_listview::{FilterSpec, OrderDirection, SortField, SortSpec};

pub use artwork::Artwork;
pub use back_button::{BackButton, DetailHeaderBar};
pub use confirm::{Confirm, ConfirmHost, use_confirm};
pub use confirm_modal::ConfirmModal;
pub use controls_shared::{SearchInput, SearchToggle, SortDropdown, dropdown_keydown};
pub use download_prefs::DownloadPrefsForm;
pub use drag_reorder::start_drag_reorder;
pub use expandable_podcast_description::ExpandablePodcastDescription;
pub use form::{
    CheckboxField, FieldError, FormErrors, FormSubmit, InputField, SelectField, SettingsRow,
    ToggleField,
};
pub use form_page::FormPage;
pub use kebab_button::KebabButton;
pub use marquee::Marquee;
// The episode-action label/icon taxonomy, shared by the per-row `episode_menu` and
// the bulk `episode_list::bulk_menu` builders (now in ui-episode-list) so the two
// can't drift — hence `pub` (was `pub(crate)` when it lived in the ui crate).
pub use menu_actions::{
    ADD_TO_PLAYLIST, ADD_TO_QUEUE, DOWNLOAD_EMBEDDED, DOWNLOAD_ON_SERVER, DOWNLOAD_TO_DEVICE,
    MenuAction, REDOWNLOAD_EMBEDDED, REDOWNLOAD_ON_DEVICE, REDOWNLOAD_ON_SERVER,
    REMOVE_DOWNLOAD_EMBEDDED, REMOVE_FROM_DEVICE, REMOVE_FROM_PLAYLIST, REMOVE_FROM_QUEUE,
    REMOVE_FROM_SERVER,
};
pub use playback_prefs::PlaybackPrefsForm;
pub use pull_to_refresh::PullToRefreshIndicator;
pub use quick_context_menu::{
    QuickAction, QuickContextMenuHost, QuickIcon, QuickMenu, use_quick_menu,
};
pub use resource_view::{resource_list_view, resource_view};
pub use rich_text::{ConfirmLinkModal, RichText};
pub use sort_search_controls::SortSearchControls;
pub use toast::ToastContainer;
