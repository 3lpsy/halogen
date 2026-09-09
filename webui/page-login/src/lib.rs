pub mod root_guard {
    pub use halogen_webui_hook_auth_redirect::*;
}
mod card;
mod embedded_setup;
mod error_message;
mod login;

pub use embedded_setup::EmbeddedServerSetup;
pub(crate) use error_message::{AuthErrorContext, auth_error_message};
pub use login::Login;
