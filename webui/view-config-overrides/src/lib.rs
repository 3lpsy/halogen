//! Admin-only online editor replaces the full override set after confirmation; clear-all also confirms. Inputs match
//! allowed field types and bounds. Save does not restart the server; View Config's restart applies changes.
mod components {
    pub use halogen_webui_component_widgets::{BackButton, ConfirmModal, FieldError};
}

mod page;
mod params;

pub use page::ConfigOverridesEdit;
