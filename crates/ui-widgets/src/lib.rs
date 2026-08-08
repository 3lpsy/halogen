#![allow(clippy::module_inception, clippy::too_many_arguments)]

//! `halogen-ui-widgets` — the Route-free, reusable presentational components:
//! forms, dialogs/menus, artwork, rich text, list controls, toast UI, etc. The
//! "dumb" view toolkit, below the Route-coupled smart components (which live in
//! `ui-views`) and consumed by `ui-episode-list` and `ui-views`.

mod artwork;
mod back_button;
mod confirm;
mod confirm_modal;
mod controls_shared;
mod download_prefs;
mod drag_reorder;
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

// Lower-layer view-data types these widgets reference (the controls/sort/filter set),
// from `halogen-ui-listview`. Crate-internal re-export so intra-crate `crate::SortSpec`
// paths resolve, without leaking through `halogen_ui_widgets::*` (ui-views re-exports
// the `halogen-ui-listview` view-data types from one explicit place instead, avoiding
// a double glob-export).
pub(crate) use halogen_ui_listview::{FilterSpec, OrderDirection, SortField, SortSpec};

pub use artwork::Artwork;
pub use back_button::{BackButton, DetailHeaderBar};
pub use confirm::{Confirm, ConfirmHost, use_confirm};
pub use confirm_modal::ConfirmModal;
pub use controls_shared::{SearchInput, SearchToggle, SortDropdown, dropdown_keydown};
pub use download_prefs::DownloadPrefsForm;
pub use drag_reorder::start_drag_reorder;
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
