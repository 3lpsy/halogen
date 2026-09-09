mod components {
    pub use halogen_webui_component_widgets::{
        BackButton, ConfirmModal, FormErrors, FormPage, FormSubmit, InputField, ToggleField,
        resource_list_view, resource_view,
    };
}

mod implementation;
pub use implementation::*;
