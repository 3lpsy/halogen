//! Shared form submit state, API-client errors, and field-error merging. Each form owns validation, offline edits, its
//! API call, and success handling; [`FormState::spawn_submit`] wraps online submission.

use dioxus::prelude::*;
use halogen_apiclient::{ApiClient, ApiError};

use halogen_webui_component_forms::FormErrors;
use halogen_webui_config::ClientConfig;

/// The canonical "no server configured" form error, the fallback a submit path sets when it can't build an API client.
/// Mirrors the message `ClientConfig::api_client_or_err` uses, routed through [`FormErrors::from_api_error`] so it
/// lands in the catch-all banner exactly as the hand-rolled version did.
pub fn no_server_configured_error() -> FormErrors {
    FormErrors::from_api_error(&ApiError::Server {
        status: 0,
        message: "No server configured.".into(),
    })
}

/// The async + error signals every form page carries: an in-flight `submitting` flag, the latest `server_errors`
/// (`None` until a submit fails), and a `submitted` flag that ungates inline field errors after the first attempt.
/// `Copy` (all fields are `Signal`s) so it can be captured by the submit closure.
#[derive(Clone, Copy)]
pub struct FormState {
    /// True while a submit request is in flight (disables the button + shows a spinner).
    pub submitting: Signal<bool>,
    /// Server-side errors from the last failed submit, folded into the form's
    /// [`FormErrors`] surface. `None` before any failure / once cleared on edit.
    pub server_errors: Signal<Option<FormErrors>>,
    /// Set on the first submit attempt — ungates "required"/local field errors so an
    /// untouched-but-empty field is flagged once the user tries to submit.
    pub submitted: Signal<bool>,
}

/// Create the [`FormState`] signal trio (`submitting=false`, `server_errors=None`,
/// `submitted=false`).
pub fn use_form_state() -> FormState {
    FormState {
        submitting: use_signal(|| false),
        server_errors: use_signal(|| Option::<FormErrors>::None),
        submitted: use_signal(|| false),
    }
}

impl FormState {
    /// Set `submitting`, build the client, call the API, and forward success to async `on_ok` or failure to
    /// `server_errors`. Every exit clears `submitting`; callers own validation and offline edits.
    pub fn spawn_submit<T, Fut, Call, OnOk, OkFut>(
        self,
        config: Signal<ClientConfig>,
        call: Call,
        on_ok: OnOk,
    ) where
        T: 'static,
        Fut: std::future::Future<Output = Result<T, ApiError>> + 'static,
        Call: FnOnce(ApiClient) -> Fut + 'static,
        OnOk: FnOnce(T) -> OkFut + 'static,
        OkFut: std::future::Future<Output = ()> + 'static,
    {
        let mut submitting = self.submitting;
        let mut server_errors = self.server_errors;
        submitting.set(true);
        let cfg = config.peek().clone();
        spawn(async move {
            let Some(client) = cfg.api_client() else {
                submitting.set(false);
                server_errors.set(Some(no_server_configured_error()));
                return;
            };
            let result = call(client).await;
            submitting.set(false);
            match result {
                Ok(value) => on_ok(value).await,
                Err(e) => server_errors.set(Some(FormErrors::from_api_error(&e))),
            }
        });
    }
}
